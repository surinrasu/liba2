use crate::PlaybackState;
use crate::audio::cipher::ChaChaPacketCipher;
use crate::audio::{AudioStreamer, LiveAudioDecoder, RtpReceiver, RtpSender};
use crate::client::control::ControlListener;
use crate::client::handshake;
use crate::client::{
    BMCA_CLOCK_ID_TIMEOUT, BMCA_YIELD_PRIORITY, DEFAULT_TRANSIENT_PIN, FEEDBACK_TIMEOUT,
    RECORD_TIMEOUT, select_best_address,
};
use crate::core::error::{Error as CoreError, RtspError};
use crate::core::stream::TimingProtocol;
use crate::core::{Device, StreamConfig, error::Result};
use crate::crypto::chacha::AudioCipher;
use crate::rtsp::{RtspConnection, RtspRequest, RtspSession, SessionState};
use crate::timing::{
    ClockOffset, NtpTimingServer, PTP_EVENT_PORT, run_bmca_yield_flow, run_ptp_slave,
};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub struct Connection {
    device: Device,
    rtsp: RtspConnection,
    session: RtspSession,
    streamer: Option<AudioStreamer>,
    playback_state: PlaybackState,
    volume_linear: f32,
    stream_config: StreamConfig,
    timing_offset: Option<ClockOffset>,
    timing_tx: Option<watch::Sender<ClockOffset>>,
    control_listener: Option<ControlListener>,
    timing_task: Option<JoinHandle<()>>,
    timing_server: Option<NtpTimingServer>,
    ptp_master_sync_task: Option<JoinHandle<()>>,
    control_receiver: Option<Arc<RtpReceiver>>,
    events_stream: Option<TcpStream>,
    ptp_master_clock_id: Option<[u8; 8]>,
    render_delay_ms: u32,
    stream_stats: Arc<crate::client::stats::StreamStats>,
}

impl Connection {
    pub(crate) fn from_paired_parts(
        device: Device,
        rtsp: RtspConnection,
        session: RtspSession,
        config: StreamConfig,
    ) -> Self {
        Self {
            device,
            rtsp,
            session,
            streamer: None,
            playback_state: PlaybackState::Stopped,
            volume_linear: 1.0,
            stream_config: config,
            timing_offset: None,
            timing_tx: None,
            timing_task: None,
            timing_server: None,
            ptp_master_sync_task: None,
            ptp_master_clock_id: None,
            control_receiver: None,
            control_listener: None,
            events_stream: None,
            render_delay_ms: 0,
            stream_stats: crate::client::stats::StreamStats::new(),
        }
    }

    pub async fn connect(device: Device, config: StreamConfig) -> Result<Self> {
        Self::connect_with_pin(device, config, DEFAULT_TRANSIENT_PIN).await
    }

    pub async fn connect_with_pin(device: Device, config: StreamConfig, pin: &str) -> Result<Self> {
        handshake::connect_with_pin(device, config, pin).await
    }

    pub async fn connect_auto(
        device: Device,
        config: StreamConfig,
        fallback_pin: &str,
    ) -> Result<Self> {
        handshake::connect_auto(device, config, fallback_pin).await
    }

    fn local_rtsp_addresses(&self) -> Vec<String> {
        self.rtsp
            .local_addr()
            .map(|addr| vec![addr.ip().to_string()])
            .unwrap_or_default()
    }

    async fn setup_rtsp_media_session(
        &mut self,
        local_timing_port: u16,
        local_addresses: Vec<String>,
        context: &'static str,
    ) -> Result<IpAddr> {
        let setup1_body = self
            .session
            .build_setup_phase1(local_timing_port, Some(local_addresses))?;
        tracing::debug!(
            context,
            uri = %self.session.request_uri(),
            body_len = setup1_body.len(),
            "Sending SETUP phase 1"
        );
        let setup1_req = RtspRequest::setup(self.session.request_uri(), setup1_body);
        let setup1_resp = self.rtsp.send(setup1_req).await?;

        if let Some(ref body) = setup1_resp.body {
            tracing::debug!(
                context,
                status = setup1_resp.status_code,
                body_len = body.len(),
                "SETUP phase 1 response received"
            );
        }

        self.session
            .process_setup_phase1_response(setup1_resp.body.as_deref().unwrap_or(&[]))?;

        let session_id = setup1_resp
            .headers
            .get("Session")
            .cloned()
            .unwrap_or_else(|| "1".to_string());
        self.rtsp.add_session_header("Session", session_id);

        let addr =
            *select_best_address(&self.device.addresses).ok_or(RtspError::ConnectionRefused)?;
        let ports = self.session.ports().ok_or_else(|| {
            CoreError::Rtsp(RtspError::InvalidResponse(
                "No ports in SETUP response".into(),
            ))
        })?;
        let event_port = ports.event_port;

        let events_addr = SocketAddr::new(addr, event_port);
        tracing::info!(context, "Establishing events connection to {}", events_addr);
        match TcpStream::connect(events_addr).await {
            Ok(stream) => {
                tracing::info!(context, "Events connection established");
                self.events_stream = Some(stream);
            }
            Err(error) => {
                warn!(
                    context,
                    "Could not connect to events port {} (proceeding anyway): {}",
                    events_addr,
                    error
                );
            }
        }

        let mut control_receiver = RtpReceiver::new();
        let actual_control_port = control_receiver.bind(0)?;
        self.session.set_local_control_port(actual_control_port);
        tracing::info!(context, "Control port bound to {}", actual_control_port);
        self.control_receiver = Some(Arc::new(control_receiver));

        let setup2_body = self.session.build_setup_phase2()?;
        let setup2_req = RtspRequest::setup(self.session.request_uri(), setup2_body);
        let setup2_resp = self.rtsp.send(setup2_req).await?;
        tracing::debug!(
            context,
            status = setup2_resp.status_code,
            body_len = setup2_resp
                .body
                .as_ref()
                .map(|body| body.len())
                .unwrap_or(0),
            "SETUP phase 2 response received"
        );

        self.session
            .process_setup_phase2_response(setup2_resp.body.as_deref().unwrap_or(&[]))?;

        self.best_effort_send_initial_record(context).await;

        Ok(addr)
    }

    async fn best_effort_send_initial_record(&mut self, context: &'static str) {
        tracing::debug!(context, "Sending RECORD request");
        let record_req = RtspRequest::record_with_info(self.session.request_uri(), 0, 0);
        match tokio::time::timeout(RECORD_TIMEOUT, self.rtsp.send(record_req)).await {
            Ok(Ok(resp)) => {
                if resp.status_code == 200 {
                    tracing::info!(context, "RECORD acknowledged");
                } else {
                    warn!(
                        context,
                        "RECORD returned status {} (continuing anyway)", resp.status_code
                    );
                }
            }
            Ok(Err(error)) => warn!(context, "RECORD error (continuing anyway): {}", error),
            Err(_) => warn!(context, "RECORD timeout (continuing anyway)"),
        }
    }

    pub async fn setup(&mut self) -> Result<()> {
        tracing::info!(
            "RTSP setup start (stream_type={:?}, timing={:?})",
            self.stream_config.stream_type,
            self.stream_config.timing_protocol
        );

        let local_timing_port = match self.stream_config.timing_protocol {
            TimingProtocol::Ntp => {
                let timing_server = NtpTimingServer::start().await?;
                let port = timing_server.port();
                tracing::info!("Started NTP timing server on port {}", port);
                self.timing_server = Some(timing_server);
                port
            }
            TimingProtocol::Ptp => {
                let mode_str = match self.stream_config.ptp_mode {
                    crate::core::PtpMode::Master => "master (sender is timing reference)",
                    crate::core::PtpMode::Slave => "slave (receiver is timing reference)",
                };
                tracing::info!("PTP timing: will act as {}", mode_str);
                PTP_EVENT_PORT
            }
        };

        let addr = self
            .setup_rtsp_media_session(local_timing_port, self.local_rtsp_addresses(), "setup")
            .await?;

        match self.stream_config.timing_protocol {
            TimingProtocol::Ntp => {
                self.timing_offset = Some(ClockOffset::default());
                tracing::info!("NTP timing: sender is reference clock (offset=0)");
            }
            TimingProtocol::Ptp => match self.stream_config.ptp_mode {
                crate::core::PtpMode::Master => {
                    let (offset_tx, mut offset_rx) = watch::channel(ClockOffset::default());
                    self.timing_tx = Some(offset_tx.clone());

                    let (clock_id_tx, clock_id_rx) = tokio::sync::oneshot::channel::<[u8; 8]>();

                    let master_ip = addr;
                    self.ptp_master_sync_task = Some(tokio::spawn(async move {
                        if let Err(e) = run_bmca_yield_flow(
                            master_ip,
                            BMCA_YIELD_PRIORITY,
                            offset_tx,
                            clock_id_tx,
                        )
                        .await
                        {
                            tracing::error!("BMCA yield flow error: {}", e);
                        }
                    }));

                    match tokio::time::timeout(BMCA_CLOCK_ID_TIMEOUT, clock_id_rx).await {
                        Ok(Ok(clock_id)) => {
                            self.ptp_master_clock_id = Some(clock_id);
                            tracing::info!("BMCA complete: HomePod clock ID = {:02x?}", clock_id);
                        }
                        Ok(Err(_)) => {
                            tracing::warn!("BMCA: clock ID channel closed unexpectedly");
                        }
                        Err(_) => {
                            tracing::warn!("BMCA: timeout waiting for clock ID (5s)");
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let initial_offset = *offset_rx.borrow_and_update();
                    self.timing_offset = Some(initial_offset);

                    tracing::info!(
                        "gPTP BMCA initialized (offset: {} ns, clock_id: {:02x?})",
                        initial_offset.offset_ns,
                        self.ptp_master_clock_id
                    );
                }
                crate::core::PtpMode::Slave => {
                    let (offset_tx, mut offset_rx) = watch::channel(ClockOffset::default());

                    self.timing_tx = Some(offset_tx.clone());

                    tracing::info!("Starting PTP slave to sync with receiver at {}", addr);
                    let ptp_task = tokio::spawn(async move {
                        match run_ptp_slave(addr, offset_tx).await {
                            Ok(()) => {
                                tracing::info!("PTP slave task completed");
                            }
                            Err(e) => {
                                tracing::error!("PTP slave task error: {}", e);
                            }
                        }
                    });

                    self.timing_task = Some(ptp_task);

                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                    let initial_offset = *offset_rx.borrow_and_update();
                    self.timing_offset = Some(initial_offset);

                    tracing::info!(
                        "PTP slave initialized (initial offset: {} ns)",
                        initial_offset.offset_ns
                    );
                }
            },
        }

        tracing::info!("RTSP setup complete");
        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut listener) = self.control_listener.take() {
            listener.stop().await;
        }

        if let Some(ref mut streamer) = self.streamer {
            streamer.stop().await?;
        }

        if let Some(task) = self.timing_task.take() {
            task.abort();
        }

        if let Some(mut server) = self.timing_server.take() {
            server.stop().await;
        }

        if let Some(task) = self.ptp_master_sync_task.take() {
            task.abort();
        }
        self.control_receiver = None;

        if self.session.state() != SessionState::Disconnected {
            if let Err(error) = self.session.start_teardown() {
                tracing::warn!(%error, "failed to mark RTSP session tearing down");
            }
            let teardown_req = RtspRequest::teardown(self.session.request_uri());
            if let Err(error) = self.rtsp.send(teardown_req).await {
                tracing::warn!(%error, "failed to send RTSP TEARDOWN");
            }
        }

        self.rtsp.close().await?;
        self.playback_state = PlaybackState::Stopped;

        Ok(())
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn playback_state(&self) -> PlaybackState {
        self.playback_state
    }

    pub fn playback_position(&self) -> f64 {
        self.streamer
            .as_ref()
            .map(|s| {
                s.position() as f64 / self.stream_config.audio_format.sample_rate.as_hz() as f64
            })
            .unwrap_or(0.0)
    }

    pub fn volume_linear(&self) -> f32 {
        self.volume_linear
    }

    pub async fn start_streaming_live(&mut self, live_decoder: LiveAudioDecoder) -> Result<()> {
        if let Some(mut listener) = self.control_listener.take() {
            listener.stop().await;
        }

        if self.session.state() != SessionState::Ready {
            self.setup().await?;
        }

        let sender = self.build_rtp_sender()?;
        info!("Live streaming: ChaCha20-Poly1305 audio encryption enabled");

        let mut streamer = AudioStreamer::new(self.stream_config.clone());
        streamer.set_rtp_sender(sender).await;
        if self.render_delay_ms > 0 {
            streamer.set_render_delay_ms(self.render_delay_ms).await;
        }
        if let Some(offset) = self.timing_offset {
            streamer.set_timing_offset(offset).await;
        }
        if let Some(ref tx) = self.timing_tx {
            streamer.set_timing_updates(tx.subscribe()).await;
        }

        self.best_effort_send_flush(0, 0).await;

        streamer.start_live(live_decoder).await?;

        self.session.start_playing()?;

        tracing::info!("Sending SET_PARAMETER volume");
        if let Err(e) = self.set_volume_linear(self.volume_linear).await {
            tracing::warn!("Failed to set volume: {}", e);
        }

        let control_listener = if let Some(ref control_rx) = self.control_receiver {
            Some(ControlListener::spawn_single(
                Arc::clone(control_rx),
                streamer.clone(),
                Arc::clone(&self.stream_stats),
            ))
        } else {
            None
        };

        self.streamer = Some(streamer);
        self.control_listener = control_listener;
        self.playback_state = PlaybackState::Playing;

        tracing::info!("Live audio streaming started");

        Ok(())
    }

    pub async fn pause(&mut self) -> Result<()> {
        if let Some(ref mut streamer) = self.streamer {
            streamer.pause().await?;
        }

        let flush_req = RtspRequest::flush_with_info(self.session.request_uri(), 0, 0);
        self.rtsp.send(flush_req).await?;

        if let Some(ref mut streamer) = self.streamer {
            streamer.reset_after_flush().await;
        }

        self.session.pause()?;
        self.playback_state = PlaybackState::Paused;

        Ok(())
    }

    pub async fn resume(&mut self) -> Result<()> {
        if let Some(ref mut streamer) = self.streamer {
            streamer.resume().await?;
        }

        let record_req = RtspRequest::record(self.session.request_uri());
        self.rtsp.send(record_req).await?;
        self.session.start_playing()?;
        self.playback_state = PlaybackState::Playing;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut listener) = self.control_listener.take() {
            listener.stop().await;
        }

        if let Some(ref mut streamer) = self.streamer {
            streamer.stop().await?;
        }

        self.best_effort_send_flush(0, 0).await;

        if let Some(ref mut streamer) = self.streamer {
            streamer.reset_after_flush().await;
        }

        self.playback_state = PlaybackState::Stopped;

        Ok(())
    }

    pub fn streamer_packets_sent(&self) -> u64 {
        self.streamer.as_ref().map_or(0, |s| s.packets_sent())
    }

    pub fn streamer_underruns(&self) -> u64 {
        self.streamer.as_ref().map_or(0, |s| s.underruns())
    }

    pub fn set_render_delay_ms(&mut self, delay_ms: u32) {
        self.render_delay_ms = delay_ms;
    }

    pub async fn set_volume_linear(&mut self, volume: f32) -> Result<()> {
        let clamped = volume.clamp(0.0, 1.0);
        let volume_db = if clamped <= 0.0 {
            -144.0
        } else {
            20.0 * clamped.log10()
        };

        self.set_volume_db(volume_db).await
    }

    pub async fn set_volume_db(&mut self, volume_db: f32) -> Result<()> {
        let clamped = volume_db.clamp(-144.0, 0.0);
        self.volume_linear = if clamped <= -144.0 {
            0.0
        } else {
            10.0_f32.powf(clamped / 20.0)
        };

        let volume_body = self.session.build_set_volume_db(clamped)?;
        let volume_req = RtspRequest::set_parameter_text(self.session.request_uri(), volume_body);
        self.rtsp.send(volume_req).await?;

        Ok(())
    }

    pub async fn send_feedback(&mut self) -> Result<()> {
        let uri = self.session.request_uri();
        let req = RtspRequest::feedback(uri);
        match tokio::time::timeout(FEEDBACK_TIMEOUT, self.rtsp.send(req)).await {
            Ok(Ok(resp)) => {
                tracing::trace!("Feedback response: status={}", resp.status_code);
                Ok(())
            }
            Ok(Err(e)) => {
                tracing::debug!("Feedback request failed: {}", e);
                Err(e)
            }
            Err(_) => {
                tracing::debug!("Feedback request timed out");
                Ok(()) // Don't fail on timeout - it's just a keepalive
            }
        }
    }

    pub async fn send_setpeers(&mut self, peer_addresses: &[String]) -> Result<()> {
        let setpeers_body = self.session.build_setpeers(peer_addresses)?;
        let setpeers_req = RtspRequest::setpeers(&self.session.id().to_string(), setpeers_body);
        self.rtsp.send(setpeers_req).await?;
        Ok(())
    }

    pub async fn setup_for_group(
        &mut self,
        ptp_clock_id: [u8; 8],
        timing_offset: ClockOffset,
        timing_rx: watch::Receiver<ClockOffset>,
    ) -> Result<()> {
        tracing::info!(
            "RTSP group setup start (stream_type={:?}, timing={:?})",
            self.stream_config.stream_type,
            self.stream_config.timing_protocol
        );

        let local_timing_port = PTP_EVENT_PORT;

        self.setup_rtsp_media_session(
            local_timing_port,
            self.local_rtsp_addresses(),
            "group_member",
        )
        .await?;

        self.ptp_master_clock_id = Some(ptp_clock_id);
        self.timing_offset = Some(timing_offset);
        let (local_tx, _) = watch::channel(timing_offset);
        self.timing_tx = Some(local_tx);
        let local_tx_clone = self.timing_tx.as_ref().unwrap().clone();
        let mut rx = timing_rx;
        self.timing_task = Some(tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let offset = *rx.borrow();
                if local_tx_clone.send(offset).is_err() {
                    tracing::debug!("No local timing subscribers left for group member");
                    break;
                }
            }
        }));

        tracing::info!(
            "Group member RTSP setup complete (clock_id={:02x?}, offset={} ns)",
            ptp_clock_id,
            timing_offset.offset_ns
        );
        Ok(())
    }

    pub async fn send_flush(&mut self, seq: u16, rtptime: u32) -> Result<()> {
        let flush_req = RtspRequest::flush_with_info(self.session.request_uri(), seq, rtptime);
        self.rtsp.send(flush_req).await?;
        tracing::info!("FLUSH sent (seq={}, rtptime={})", seq, rtptime);
        Ok(())
    }

    pub(crate) async fn best_effort_send_flush(&mut self, seq: u16, rtptime: u32) {
        if let Err(error) = self.send_flush(seq, rtptime).await {
            tracing::warn!(
                %error,
                device = %self.device.name,
                seq,
                rtptime,
                "FLUSH failed (continuing anyway)"
            );
        }
    }

    pub async fn send_record(&mut self) -> Result<()> {
        let record_req = RtspRequest::record(self.session.request_uri());
        self.rtsp.send(record_req).await?;
        tracing::info!("RECORD sent");
        Ok(())
    }

    pub(crate) async fn best_effort_send_record(&mut self) {
        if let Err(error) = self.send_record().await {
            tracing::warn!(
                %error,
                device = %self.device.name,
                "RECORD failed (continuing anyway)"
            );
        }
    }

    pub fn build_rtp_sender(&self) -> Result<RtpSender> {
        let ports = self
            .session
            .ports()
            .ok_or_else(|| RtspError::SetupFailed("Missing ports from SETUP".into()))?;
        let dest_addr =
            select_best_address(&self.device.addresses).ok_or(RtspError::ConnectionRefused)?;
        let dest = SocketAddr::new(*dest_addr, ports.data_port);
        let control_dest = SocketAddr::new(*dest_addr, ports.control_port);

        let mut sender = RtpSender::new(dest, 0); // SSRC=0 per AirPlay spec
        sender.set_control_dest(control_dest);
        sender.bind(0)?;

        if let Some(ref control_rx) = self.control_receiver
            && let Ok(Some(ctrl_sock)) = control_rx.try_clone_socket()
        {
            let ctrl_port = ctrl_sock.local_addr().map(|a| a.port()).unwrap_or(0);
            sender.set_control_socket(ctrl_sock);
            tracing::info!("RTP sender: sync packets from control port {}", ctrl_port);
        }

        let stream_key = *self.session.stream_key();
        let audio_cipher = AudioCipher::new(stream_key);
        sender.set_cipher(Box::new(ChaChaPacketCipher::new(audio_cipher)));

        tracing::info!("Built RTP sender: data={}, control={}", dest, control_dest);
        Ok(sender)
    }

    pub fn ptp_master_clock_id(&self) -> Option<[u8; 8]> {
        self.ptp_master_clock_id
    }

    pub fn timing_offset(&self) -> Option<ClockOffset> {
        self.timing_offset
    }

    pub fn timing_rx(&self) -> Option<watch::Receiver<ClockOffset>> {
        self.timing_tx.as_ref().map(|tx| tx.subscribe())
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.rtsp.local_addr()
    }

    pub fn stream_stats(&self) -> Arc<crate::client::stats::StreamStats> {
        Arc::clone(&self.stream_stats)
    }

    pub fn clone_control_socket_for_recv(&self) -> Option<UdpSocket> {
        self.control_receiver
            .as_ref()
            .and_then(|rx| rx.try_clone_socket().ok().flatten())
    }
}
