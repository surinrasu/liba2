use crate::PlaybackState;
use crate::audio::{AudioStreamer, LiveAudioDecoder};
use crate::client::Connection;
use crate::client::control::ControlListener;
use crate::client::stats::{StatsSnapshot, StreamStats};
use crate::core::StreamConfig;
use crate::core::error::{Result, RtspError};
use std::net::UdpSocket;
use std::sync::Arc;

pub(crate) struct GroupSession {
    connections: Vec<Connection>,
    streamer: Option<AudioStreamer>,
    playback_state: Option<PlaybackState>,
    stream_stats: Arc<StreamStats>,
    control_listener: Option<ControlListener>,
}

impl GroupSession {
    pub(crate) fn new(connections: Vec<Connection>) -> Self {
        Self {
            connections,
            streamer: None,
            playback_state: None,
            stream_stats: StreamStats::new(),
            control_listener: None,
        }
    }

    pub(crate) async fn disconnect(&mut self) -> Result<()> {
        self.stop_control_listener().await;

        if let Some(ref mut streamer) = self.streamer
            && let Err(error) = streamer.stop().await
        {
            tracing::warn!(%error, "failed to stop group streamer during disconnect");
        }
        self.streamer = None;

        for (index, connection) in self.connections.iter_mut().enumerate() {
            if let Err(error) = connection.disconnect().await {
                tracing::warn!(%error, index, "failed to disconnect group member");
            }
        }
        self.connections.clear();
        self.playback_state = None;

        Ok(())
    }

    pub(crate) async fn pause(&mut self, primary: &mut Connection) -> Result<()> {
        if let Some(ref mut streamer) = self.streamer {
            streamer.pause().await?;
        }

        primary.pause().await?;

        for connection in &mut self.connections {
            connection.best_effort_send_flush(0, 0).await;
        }

        if let Some(ref mut streamer) = self.streamer {
            streamer.reset_after_flush().await;
        }

        if self.playback_state.is_some() {
            self.playback_state = Some(PlaybackState::Paused);
        }
        Ok(())
    }

    pub(crate) async fn resume(&mut self, primary: &mut Connection) -> Result<()> {
        primary.resume().await?;

        for connection in &mut self.connections {
            connection.best_effort_send_record().await;
        }

        if let Some(ref mut streamer) = self.streamer {
            streamer.resume().await?;
        }

        if self.playback_state.is_some() {
            self.playback_state = Some(PlaybackState::Playing);
        }
        Ok(())
    }

    pub(crate) async fn stop(&mut self, primary: &mut Connection) -> Result<()> {
        self.stop_control_listener().await;

        if let Some(ref mut streamer) = self.streamer
            && let Err(error) = streamer.stop().await
        {
            tracing::warn!(%error, "failed to stop group streamer");
        }
        self.streamer = None;

        primary.stop().await?;

        for connection in &mut self.connections {
            connection.best_effort_send_flush(0, 0).await;
        }

        self.playback_state = None;
        Ok(())
    }

    pub(crate) async fn set_volume_linear(&mut self, volume: f32) -> Result<()> {
        for (index, connection) in self.connections.iter_mut().enumerate() {
            if let Err(error) = connection.set_volume_linear(volume).await {
                tracing::warn!(%error, index, "failed to set group member volume");
            }
        }
        Ok(())
    }

    pub(crate) async fn set_volume_db(&mut self, volume_db: f32) -> Result<()> {
        for (index, connection) in self.connections.iter_mut().enumerate() {
            if let Err(error) = connection.set_volume_db(volume_db).await {
                tracing::warn!(%error, index, "failed to set group member volume");
            }
        }
        Ok(())
    }

    pub(crate) async fn send_feedback(&mut self) -> Result<()> {
        for (index, connection) in self.connections.iter_mut().enumerate() {
            if let Err(error) = connection.send_feedback().await {
                tracing::warn!(%error, index, "failed to send group member feedback");
            }
        }
        Ok(())
    }

    pub(crate) async fn start_live_streaming(
        &mut self,
        primary: &mut Connection,
        stream_config: &StreamConfig,
        render_delay_ms: u32,
        decoder: LiveAudioDecoder,
    ) -> Result<()> {
        self.stop_control_listener().await;

        if let Some(ref mut streamer) = self.streamer
            && let Err(error) = streamer.stop().await
        {
            tracing::warn!(%error, "failed to stop existing group streamer");
        }
        self.streamer = None;

        primary.best_effort_send_flush(0, 0).await;
        for connection in &mut self.connections {
            connection.best_effort_send_flush(0, 0).await;
        }

        let volume = primary.volume_linear();
        tracing::info!("Sending SET_PARAMETER volume to AirPlay group");
        if let Err(error) = primary.set_volume_linear(volume).await {
            tracing::warn!(%error, "failed to set AirPlay group primary volume");
        }
        self.set_volume_linear(volume).await?;

        let mut senders = Vec::new();
        senders.push(primary.build_rtp_sender()?);
        for connection in &self.connections {
            senders.push(connection.build_rtp_sender()?);
        }

        let ptp_clock_id = primary
            .ptp_master_clock_id()
            .ok_or_else(|| RtspError::SetupFailed("No PTP clock ID for group streaming".into()))?;

        let mut streamer = AudioStreamer::new(stream_config.clone());
        streamer.set_rtp_senders(senders).await;

        if let Some(offset) = primary.timing_offset() {
            streamer.set_timing_offset(offset).await;
        }
        if let Some(rx) = primary.timing_rx() {
            streamer.set_timing_updates(rx).await;
        }
        streamer.set_ptp_sync_mode(ptp_clock_id).await;
        if render_delay_ms > 0 {
            streamer.set_render_delay_ms(render_delay_ms).await;
        }

        streamer.start_live(decoder).await?;

        let device_count = 1 + self.connections.len();
        self.stream_stats = StreamStats::with_device_count(device_count);
        self.control_listener = self.spawn_control_listener(primary, &streamer);

        self.streamer = Some(streamer);
        self.playback_state = Some(PlaybackState::Playing);

        Ok(())
    }

    pub(crate) fn playback_state(&self) -> Option<PlaybackState> {
        self.playback_state
    }

    pub(crate) fn is_connected(&self) -> bool {
        !self.connections.is_empty()
    }

    pub(crate) fn device_count(&self) -> usize {
        if self.connections.is_empty() {
            0
        } else {
            1 + self.connections.len()
        }
    }

    pub(crate) fn stats_snapshot(&self) -> StatsSnapshot {
        let mut snapshot = self.stream_stats.snapshot();
        if let Some(ref streamer) = self.streamer {
            snapshot.packets_sent = streamer.packets_sent();
            snapshot.underruns = streamer.underruns();
        }
        snapshot
    }

    fn spawn_control_listener(
        &self,
        primary: &Connection,
        streamer: &AudioStreamer,
    ) -> Option<ControlListener> {
        let mut control_sockets: Vec<(usize, UdpSocket)> = Vec::new();

        if let Some(socket) = primary.clone_control_socket_for_recv() {
            control_sockets.push((0, socket));
        }

        for (index, connection) in self.connections.iter().enumerate() {
            if let Some(socket) = connection.clone_control_socket_for_recv() {
                control_sockets.push((index + 1, socket));
            }
        }

        ControlListener::spawn_group(
            control_sockets,
            streamer.clone(),
            Arc::clone(&self.stream_stats),
        )
    }

    async fn stop_control_listener(&mut self) {
        if let Some(mut listener) = self.control_listener.take() {
            listener.stop().await;
        }
    }
}
