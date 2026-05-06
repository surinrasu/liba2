use crate::PlaybackState;
use crate::audio::{AlacEncoder, LiveAudioDecoder, LiveFrameSender};
use crate::client::group::GroupSession;
use crate::client::{Connection, StatsSnapshot};
use crate::client::{
    DEFAULT_RENDER_DELAY_MS, DEFAULT_TRANSIENT_PIN, LIVE_FRAME_CHANNEL_CAPACITY,
    PLAYBACK_COMPLETION_POLL_INTERVAL,
};
use crate::core::error::{DiscoveryError, Error, RtspError};
use crate::core::{Device, DeviceId, StreamConfig, error::Result};
use crate::discovery::{Discovery, ServiceBrowser};
use std::time::Duration;

pub struct AirPlayClient {
    browser: ServiceBrowser,
    connection: Option<Connection>,
    group: Option<GroupSession>,
    stream_config: StreamConfig,
    render_delay_ms: u32,
}

impl AirPlayClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            browser: ServiceBrowser::new()?,
            connection: None,
            group: None,
            stream_config: StreamConfig::default(),
            render_delay_ms: DEFAULT_RENDER_DELAY_MS,
        })
    }

    pub fn with_config(config: StreamConfig) -> Result<Self> {
        Ok(Self {
            browser: ServiceBrowser::new()?,
            connection: None,
            group: None,
            stream_config: config,
            render_delay_ms: DEFAULT_RENDER_DELAY_MS,
        })
    }

    pub fn set_render_delay_ms(&mut self, delay_ms: u32) {
        self.render_delay_ms = delay_ms;
    }

    pub async fn discover(&self, timeout: Duration) -> Result<Vec<Device>> {
        self.browser.scan(timeout).await
    }

    pub async fn get_device(&self, id: &DeviceId) -> Option<Device> {
        self.browser.get_device(id).await
    }

    pub async fn connect(&mut self, device: &Device) -> Result<()> {
        if self.connection.is_some() {
            self.disconnect().await?;
        }

        let stream_config = self.stream_config.clone();

        let mut connection = Connection::connect(device.clone(), stream_config).await?;

        connection.set_render_delay_ms(self.render_delay_ms);

        connection.setup().await?;

        self.connection = Some(connection);

        Ok(())
    }

    pub async fn connect_with_pin(&mut self, device: &Device, pin: &str) -> Result<()> {
        if self.connection.is_some() {
            self.disconnect().await?;
        }

        let stream_config = self.stream_config.clone();

        let mut connection =
            Connection::connect_with_pin(device.clone(), stream_config, pin).await?;

        connection.set_render_delay_ms(self.render_delay_ms);

        connection.setup().await?;

        self.connection = Some(connection);

        Ok(())
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(ref mut group) = self.group {
            group.disconnect().await?;
        }
        self.group = None;

        if let Some(ref mut connection) = self.connection {
            connection.disconnect().await?;
        }
        self.connection = None;

        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn connected_device(&self) -> Option<&Device> {
        self.connection.as_ref().map(|c| c.device())
    }

    pub async fn start_live_streaming(
        &mut self,
        sample_rate: u32,
        channels: u8,
    ) -> Result<LiveFrameSender> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        let (sender, decoder) =
            LiveAudioDecoder::create_pair(sample_rate, channels, LIVE_FRAME_CHANNEL_CAPACITY);

        connection.start_streaming_live(decoder).await?;

        Ok(sender)
    }

    pub async fn start_live_streaming_with_decoder(
        &mut self,
        decoder: LiveAudioDecoder,
    ) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        connection.start_streaming_live(decoder).await
    }

    pub async fn pause(&mut self) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        if let Some(group) = self.group.as_mut() {
            group.pause(connection).await?;
        } else {
            connection.pause().await?;
        }
        Ok(())
    }

    pub async fn resume(&mut self) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        if let Some(group) = self.group.as_mut() {
            group.resume(connection).await?;
        } else {
            connection.resume().await?;
        }
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        if let Some(group) = self.group.as_mut() {
            group.stop(connection).await?;
        } else {
            connection.stop().await?;
        }
        Ok(())
    }

    pub async fn set_volume_linear(&mut self, volume: f32) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        connection.set_volume_linear(volume).await?;

        if let Some(group) = self.group.as_mut() {
            group.set_volume_linear(volume).await?;
        }

        Ok(())
    }

    pub async fn set_volume_db(&mut self, volume_db: f32) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        connection.set_volume_db(volume_db).await?;

        if let Some(group) = self.group.as_mut() {
            group.set_volume_db(volume_db).await?;
        }

        Ok(())
    }

    pub async fn send_feedback(&mut self) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        connection.send_feedback().await?;

        if let Some(group) = self.group.as_mut() {
            group.send_feedback().await?;
        }

        Ok(())
    }

    pub fn playback_state(&self) -> PlaybackState {
        if let Some(group) = self.group.as_ref()
            && let Some(state) = group.playback_state()
        {
            return state;
        }
        self.connection
            .as_ref()
            .map(|c| c.playback_state())
            .unwrap_or(PlaybackState::Stopped)
    }

    pub fn playback_position(&self) -> f64 {
        self.connection
            .as_ref()
            .map(|c| c.playback_position())
            .unwrap_or(0.0)
    }

    pub async fn wait_for_completion(&self) -> Result<()> {
        loop {
            let state = self.playback_state();
            match state {
                PlaybackState::Stopped | PlaybackState::Error => break,
                _ => {
                    tokio::time::sleep(PLAYBACK_COMPLETION_POLL_INTERVAL).await;
                }
            }
        }
        Ok(())
    }

    pub async fn connect_group(&mut self, devices: &[Device]) -> Result<()> {
        if devices.len() < 2 {
            return Err(Error::Discovery(DiscoveryError::NoDevicesFound));
        }

        self.disconnect().await?;

        let mut config = self.stream_config.clone();
        config.timing_protocol = crate::core::stream::TimingProtocol::Ptp;
        config.ptp_mode = crate::core::PtpMode::Master;

        if config.audio_format.codec == crate::core::AudioCodec::Alac && config.asc.is_none() {
            let temp_encoder = AlacEncoder::new(config.audio_format).map_err(|e| {
                Error::Streaming(crate::core::error::StreamingError::Encoding(format!(
                    "Failed to create encoder for magic cookie: {}",
                    e
                )))
            })?;
            config.asc = Some(temp_encoder.magic_cookie());
        }

        let mut connections: Vec<Connection> = Vec::new();
        for device in devices {
            let conn =
                Connection::connect_auto(device.clone(), config.clone(), DEFAULT_TRANSIENT_PIN)
                    .await?;
            connections.push(conn);
        }

        let mut peer_addresses: Vec<String> = devices
            .iter()
            .flat_map(|d| {
                d.addresses
                    .iter()
                    .find(|a| a.is_ipv4())
                    .map(|a| a.to_string())
            })
            .collect();
        if let Some(local_addr) = connections[0].local_addr() {
            peer_addresses.push(local_addr.ip().to_string());
        }

        connections[0].setup().await?;
        connections[0].set_render_delay_ms(self.render_delay_ms);
        connections[0].send_setpeers(&peer_addresses).await?;

        let ptp_clock_id = connections[0]
            .ptp_master_clock_id()
            .ok_or_else(|| RtspError::SetupFailed("Primary has no PTP clock ID".into()))?;
        let timing_offset = connections[0]
            .timing_offset()
            .ok_or_else(|| RtspError::SetupFailed("Primary has no timing offset".into()))?;
        let timing_rx = connections[0]
            .timing_rx()
            .ok_or_else(|| RtspError::SetupFailed("Primary has no timing channel".into()))?;

        for connection in connections.iter_mut().skip(1) {
            let rx_clone = timing_rx.clone();
            connection
                .setup_for_group(ptp_clock_id, timing_offset, rx_clone)
                .await?;
            connection.set_render_delay_ms(self.render_delay_ms);
            connection.send_setpeers(&peer_addresses).await?;
        }

        let mut iter = connections.into_iter();
        self.connection = iter.next();
        self.group = Some(GroupSession::new(iter.collect()));

        Ok(())
    }

    pub async fn start_group_live_streaming(
        &mut self,
        sample_rate: u32,
        channels: u8,
    ) -> Result<LiveFrameSender> {
        let (sender, decoder) =
            LiveAudioDecoder::create_pair(sample_rate, channels, LIVE_FRAME_CHANNEL_CAPACITY);

        self.start_group_live_streaming_with_decoder(decoder)
            .await?;

        Ok(sender)
    }

    pub async fn start_group_live_streaming_with_decoder(
        &mut self,
        decoder: LiveAudioDecoder,
    ) -> Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;
        let group = self
            .group
            .as_mut()
            .ok_or(Error::Rtsp(RtspError::NoSession))?;

        group
            .start_live_streaming(
                connection,
                &self.stream_config,
                self.render_delay_ms,
                decoder,
            )
            .await
    }

    pub fn is_group_connected(&self) -> bool {
        self.group.as_ref().is_some_and(GroupSession::is_connected)
    }

    pub fn group_device_count(&self) -> usize {
        self.group.as_ref().map_or(0, GroupSession::device_count)
    }

    pub fn stats_snapshot(&self) -> StatsSnapshot {
        if let Some(group) = self.group.as_ref() {
            group.stats_snapshot()
        } else if let Some(ref conn) = self.connection {
            let mut snap = conn.stream_stats().snapshot();
            snap.packets_sent = conn.streamer_packets_sent();
            snap.underruns = conn.streamer_underruns();
            snap
        } else {
            StatsSnapshot::default()
        }
    }
}
