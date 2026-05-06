use crate::core::error::{Result, RtspError};
use crate::core::stream::TimingProtocol;
use crate::core::{Device, StreamConfig};
use uuid::Uuid;

use crate::rtsp::plist_codec::{
    self, SetupPhase1Request, SetupPhase1Response, SetupPhase2Request, SetupPhase2Response,
    StreamDef, TimingPeerInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    Connected,
    Paired,
    SetupPhase1,
    Ready,
    Playing,
    Paused,
    TearingDown,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionPorts {
    pub data_port: u16,
    pub control_port: u16,
    pub event_port: u16,
}

pub struct RtspSession {
    session_id: Uuid,
    state: SessionState,
    client_device_id: String,
    stream_config: StreamConfig,
    ports: Option<SessionPorts>,
    local_control_port: u16,
    stream_connection_id: u32,
    shk: [u8; 32],
    request_host: Option<String>,
}

impl RtspSession {
    pub fn new(device: Device, stream_config: StreamConfig) -> Self {
        let mut shk = [0u8; 32];
        getrandom::fill(&mut shk).expect("operating system random source failed");

        let local_control_port = 49152
            + (getrandom::u32().expect("operating system random source failed") % 16383) as u16;

        let stream_connection_id = getrandom::u32().expect("operating system random source failed");

        let client_device_id = device.id.to_mac_string();

        Self {
            session_id: Uuid::new_v4(),
            state: SessionState::Disconnected,
            client_device_id,
            stream_config,
            ports: None,
            local_control_port,
            stream_connection_id,
            shk,
            request_host: None,
        }
    }

    pub fn id(&self) -> Uuid {
        self.session_id
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn set_client_device_id(&mut self, device_id: String) {
        self.client_device_id = device_id;
    }

    pub fn set_request_host(&mut self, host: String) {
        self.request_host = Some(host);
    }

    pub fn set_local_control_port(&mut self, port: u16) {
        self.local_control_port = port;
    }

    pub fn request_uri(&self) -> String {
        let host = self.request_host.as_deref().unwrap_or("local");
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{}]", host)
        } else {
            host.to_string()
        };
        format!(
            "rtsp://{}/{}",
            host,
            self.session_id.to_string().to_uppercase()
        )
    }

    pub fn ports(&self) -> Option<&SessionPorts> {
        self.ports.as_ref()
    }

    pub fn stream_key(&self) -> &[u8; 32] {
        &self.shk
    }

    pub fn set_connected(&mut self) -> Result<()> {
        if self.state != SessionState::Disconnected {
            return Err(RtspError::SetupFailed(format!(
                "Cannot connect from state {:?}",
                self.state
            ))
            .into());
        }
        self.state = SessionState::Connected;
        Ok(())
    }

    pub fn set_paired(&mut self) -> Result<()> {
        if self.state != SessionState::Connected {
            return Err(
                RtspError::SetupFailed(format!("Cannot pair from state {:?}", self.state)).into(),
            );
        }
        self.state = SessionState::Paired;
        Ok(())
    }

    pub fn build_setup_phase1(
        &self,
        local_timing_port: u16,
        local_addresses: Option<Vec<String>>,
    ) -> Result<Vec<u8>> {
        let timing_protocol = match self.stream_config.timing_protocol {
            TimingProtocol::Ntp => "NTP",
            TimingProtocol::Ptp => "PTP",
        };

        let (timing_peer_info, timing_peer_list) =
            if self.stream_config.timing_protocol == TimingProtocol::Ptp {
                let addresses = local_addresses.unwrap_or_default();

                let peer_info = TimingPeerInfo {
                    addresses,
                    id: self.session_id.to_string(),
                    supports_clock_port_matching_override: true,
                };
                let peer_list = vec![peer_info.clone()];
                (Some(peer_info), Some(peer_list))
            } else {
                (None, None)
            };

        let request = SetupPhase1Request {
            device_id: self.client_device_id.clone(),
            session_uuid: self.session_id.to_string().to_uppercase(),
            timing_port: local_timing_port,
            timing_protocol: timing_protocol.to_string(),
            timing_peer_info,
            timing_peer_list,
        };

        tracing::debug!(
            device_id = %request.device_id,
            session_uuid = %request.session_uuid,
            timing_port = request.timing_port,
            timing_protocol = %request.timing_protocol,
            has_timing_peer_info = request.timing_peer_info.is_some(),
            "SETUP phase1 payload (minimal owntone-style)"
        );

        let encoded = plist_codec::encode(&request)?;
        if let Ok(dict) = plist_codec::decode::<plist::Dictionary>(&encoded) {
            tracing::debug!(
                "SETUP phase1 plist keys: {:?}",
                dict.keys().collect::<Vec<_>>()
            );
            for (key, value) in dict.iter() {
                match value {
                    plist::Value::String(s) => tracing::debug!("  {}: \"{}\"", key, s),
                    plist::Value::Integer(i) => {
                        tracing::debug!("  {}: {}", key, i.as_unsigned().unwrap_or(0))
                    }
                    _ => tracing::debug!("  {}: {:?}", key, value),
                }
            }
        }
        let hex: String = encoded
            .iter()
            .take(64)
            .map(|b| format!("{:02x}", b))
            .collect();
        tracing::debug!("SETUP phase1 plist hex (first 64 bytes): {}", hex);

        Ok(encoded)
    }

    pub fn process_setup_phase1_response(&mut self, response: &[u8]) -> Result<()> {
        if self.state != SessionState::Paired {
            return Err(RtspError::SetupFailed(format!(
                "Cannot process phase 1 from state {:?}",
                self.state
            ))
            .into());
        }

        let phase1_response: SetupPhase1Response = plist_codec::decode(response)?;

        tracing::info!(
            "SETUP phase 1 response: event_port={}, timing_port={}, has_timing_peer_info={}",
            phase1_response.event_port,
            phase1_response.timing_port,
            phase1_response.timing_peer_info.is_some()
        );

        if let Some(ref peer_info) = phase1_response.timing_peer_info {
            tracing::info!(
                "Receiver timing peer info: addresses={:?}, id={}",
                peer_info.addresses,
                peer_info.id
            );
            if self.stream_config.timing_protocol == TimingProtocol::Ptp
                && phase1_response.timing_port != 0
            {
                tracing::warn!(
                    "Receiver reported timing_port={} for PTP (expected 0)",
                    phase1_response.timing_port
                );
            }
        }

        self.ports = Some(SessionPorts {
            data_port: 0,    // Will be set in phase 2
            control_port: 0, // Will be set in phase 2
            event_port: phase1_response.event_port,
        });

        self.state = SessionState::SetupPhase1;
        Ok(())
    }

    pub fn build_setup_phase2(&self) -> Result<Vec<u8>> {
        use crate::core::codec::AudioCodec;

        if self.state != SessionState::SetupPhase1 {
            return Err(RtspError::SetupFailed(format!(
                "Cannot build phase 2 from state {:?}",
                self.state
            ))
            .into());
        }

        let asc = if let Some(ref asc) = self.stream_config.asc {
            Some(asc.clone())
        } else {
            match self.stream_config.audio_format.codec {
                AudioCodec::Aac | AudioCodec::AacEld => Some(vec![0x12, 0x10]),
                _ => None,
            }
        };

        let stream_def = StreamDef {
            stream_type: self.stream_config.stream_type as u32,
            audio_format: self.stream_config.audio_format.codec.audio_format_value(),
            audio_mode: "default".to_string(),
            sample_rate: self.stream_config.audio_format.sample_rate.as_hz(),
            ct: self.stream_config.audio_format.codec.compression_type(),
            control_port: self.local_control_port,
            is_media: true,
            latency_min: self.stream_config.latency_min,
            latency_max: self.stream_config.latency_max,
            shk: self.shk.to_vec(),
            asc,
            spf: self.stream_config.audio_format.frames_per_packet,
            supports_dynamic_stream_id: self.stream_config.supports_dynamic_stream_id,
            stream_connection_id: self.stream_connection_id,
        };

        let request = SetupPhase2Request {
            streams: vec![stream_def],
        };

        tracing::debug!(
            stream_type = request.streams[0].stream_type,
            audio_format = request.streams[0].audio_format,
            sample_rate = request.streams[0].sample_rate,
            ct = request.streams[0].ct,
            control_port = request.streams[0].control_port,
            latency_min = request.streams[0].latency_min,
            latency_max = request.streams[0].latency_max,
            spf = request.streams[0].spf,
            has_asc = request.streams[0].asc.is_some(),
            asc = ?request.streams[0].asc,
            stream_connection_id = self.stream_connection_id,
            "SETUP phase2 payload"
        );
        tracing::debug!("SETUP phase2 shk (first 8 bytes): {:02x?}", &self.shk[..8]);

        plist_codec::encode(&request)
    }

    pub fn process_setup_phase2_response(&mut self, response: &[u8]) -> Result<()> {
        if self.state != SessionState::SetupPhase1 {
            return Err(RtspError::SetupFailed(format!(
                "Cannot process phase 2 from state {:?}",
                self.state
            ))
            .into());
        }

        let phase2_response: SetupPhase2Response = plist_codec::decode(response)?;

        if let Ok(raw) = plist::from_bytes::<plist::Dictionary>(response) {
            tracing::debug!("SETUP phase2 response raw: {:?}", raw);
        }

        let audio_stream = phase2_response
            .streams
            .iter()
            .find(|s| s.stream_type == self.stream_config.stream_type as u32)
            .ok_or_else(|| {
                RtspError::SetupFailed("No matching stream in phase 2 response".to_string())
            })?;

        tracing::debug!(
            "SETUP phase2 response stream: type={}, data_port={}, control_port={}, stream_id={}",
            audio_stream.stream_type,
            audio_stream.data_port,
            audio_stream.control_port,
            audio_stream.stream_id
        );

        if let Some(ref mut ports) = self.ports {
            ports.data_port = audio_stream.data_port;
            ports.control_port = audio_stream.control_port;
        }

        self.state = SessionState::Ready;
        Ok(())
    }

    pub fn start_playing(&mut self) -> Result<()> {
        if self.state != SessionState::Ready && self.state != SessionState::Paused {
            return Err(RtspError::SetupFailed(format!(
                "Cannot start playing from state {:?}",
                self.state
            ))
            .into());
        }
        self.state = SessionState::Playing;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        if self.state != SessionState::Playing {
            return Err(RtspError::SetupFailed(format!(
                "Cannot pause from state {:?}",
                self.state
            ))
            .into());
        }
        self.state = SessionState::Paused;
        Ok(())
    }

    pub fn start_teardown(&mut self) -> Result<()> {
        match self.state {
            SessionState::Disconnected => {
                return Err(RtspError::SetupFailed("Not connected".to_string()).into());
            }
            SessionState::TearingDown => {
                return Err(RtspError::SetupFailed("Already tearing down".to_string()).into());
            }
            _ => {}
        }
        self.state = SessionState::TearingDown;
        Ok(())
    }

    pub fn build_set_volume_db(&self, volume_db: f32) -> Result<Vec<u8>> {
        Ok(format!("volume: {:.2}\r\n", volume_db).into_bytes())
    }

    pub fn build_setpeers(&self, peer_addresses: &[String]) -> Result<Vec<u8>> {
        plist_codec::encode(&peer_addresses.to_vec())
    }
}
