use crate::core::codec::AudioFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamType {
    Realtime = 96,
    Buffered = 103,
    Mirror = 110,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimingProtocol {
    Ntp,
    Ptp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PtpMode {
    Master,
    Slave,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StreamConfig {
    pub stream_type: StreamType,
    pub audio_format: AudioFormat,
    pub timing_protocol: TimingProtocol,
    pub ptp_mode: PtpMode,
    pub latency_min: u32,
    pub latency_max: u32,
    pub supports_dynamic_stream_id: bool,
    pub asc: Option<Vec<u8>>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            stream_type: StreamType::Realtime,
            audio_format: AudioFormat::default(),
            timing_protocol: TimingProtocol::Ntp,
            ptp_mode: PtpMode::Master, // Default to master for backward compatibility
            latency_min: 11025,        // ~250ms at 44.1kHz
            latency_max: 88200,        // ~2s at 44.1kHz
            supports_dynamic_stream_id: true,
            asc: None,
        }
    }
}

impl StreamConfig {
    pub fn airplay2_buffered() -> Self {
        Self {
            stream_type: StreamType::Buffered,
            audio_format: AudioFormat::buffered_default(),
            timing_protocol: TimingProtocol::Ptp,
            ptp_mode: PtpMode::Master,
            latency_min: 22050,  // ~500ms
            latency_max: 132300, // ~3s
            supports_dynamic_stream_id: true,
            asc: None,
        }
    }

    pub fn airplay2_buffered_ntp() -> Self {
        Self {
            stream_type: StreamType::Buffered,
            audio_format: AudioFormat::buffered_default(),
            timing_protocol: TimingProtocol::Ntp,
            ptp_mode: PtpMode::Master, // Doesn't matter for NTP, but set for consistency
            latency_min: 22050,        // ~500ms
            latency_max: 132300,       // ~3s
            supports_dynamic_stream_id: true,
            asc: None,
        }
    }

    pub fn airplay1_realtime() -> Self {
        Self::default()
    }

    pub fn latency_min_ms(&self) -> u32 {
        (self.latency_min as u64 * 1000 / self.audio_format.sample_rate.as_hz() as u64) as u32
    }

    pub fn latency_max_ms(&self) -> u32 {
        (self.latency_max as u64 * 1000 / self.audio_format.sample_rate.as_hz() as u64) as u32
    }
}
