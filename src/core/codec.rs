use crate::protocol::{
    AIRPLAY_AAC_FRAMES_PER_PACKET, AIRPLAY_ALAC_FRAMES_PER_PACKET, SAMPLE_RATE_44100,
    SAMPLE_RATE_48000,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AudioCodec {
    Pcm,
    Alac,
    Aac,
    AacEld,
    Opus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleRate {
    Hz44100,
    Hz48000,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AudioFormat {
    pub codec: AudioCodec,
    pub sample_rate: SampleRate,
    pub bit_depth: u8,
    pub channels: u8,
    pub frames_per_packet: u32,
}

impl AudioCodec {
    pub fn compression_type(&self) -> u8 {
        match self {
            Self::Pcm => 1,
            Self::Alac => 2,
            Self::Aac => 3,
            Self::AacEld => 4,
            Self::Opus => 32,
        }
    }

    pub fn audio_format_value(&self) -> u32 {
        match self {
            Self::Pcm => 0,            // PCM doesn't have a specific audioFormat value
            Self::Alac => 0x40000,     // 262144
            Self::Aac => 0x400000,     // 4194304
            Self::AacEld => 0x1000000, // 16777216
            Self::Opus => 0x2000000,   // 33554432 (estimated)
        }
    }

    pub fn from_compression_type(ct: u8) -> Option<Self> {
        match ct {
            1 => Some(Self::Pcm),
            2 => Some(Self::Alac),
            3 => Some(Self::Aac),
            4 => Some(Self::AacEld),
            32 => Some(Self::Opus),
            _ => None,
        }
    }
}

impl SampleRate {
    pub fn as_hz(&self) -> u32 {
        match self {
            Self::Hz44100 => SAMPLE_RATE_44100,
            Self::Hz48000 => SAMPLE_RATE_48000,
        }
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Alac,
            sample_rate: SampleRate::Hz44100,
            bit_depth: 16,
            channels: 2,
            frames_per_packet: AIRPLAY_ALAC_FRAMES_PER_PACKET,
        }
    }
}

impl AudioFormat {
    pub fn realtime_default() -> Self {
        Self::default()
    }

    pub fn buffered_default() -> Self {
        Self {
            codec: AudioCodec::Aac,
            sample_rate: SampleRate::Hz44100,
            bit_depth: 16,
            channels: 2,
            frames_per_packet: AIRPLAY_AAC_FRAMES_PER_PACKET,
        }
    }

    pub fn bytes_per_frame(&self) -> usize {
        (self.bit_depth as usize * self.channels as usize) / 8
    }

    pub fn buffer_size_for_duration_ms(&self, ms: u32) -> usize {
        let samples = (self.sample_rate.as_hz() as u64 * ms as u64) / 1000;
        samples as usize * self.bytes_per_frame()
    }
}
