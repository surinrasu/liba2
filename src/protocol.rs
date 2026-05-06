pub(crate) const AIRPLAY_USER_AGENT: &str = "AirPlay/745.83";
pub(crate) const AIRPLAY_CLIENT_NAME: &str = "Rust AirPlay Sender";
pub(crate) const AIRPLAY_ACTIVE_REMOTE: &str = "1234567890";

pub(crate) const HKP_TRANSIENT: u8 = 4;

pub(crate) const SAMPLE_RATE_44100: u32 = 44_100;
pub(crate) const SAMPLE_RATE_48000: u32 = 48_000;
pub(crate) const AIRPLAY_ALAC_FRAMES_PER_PACKET: u32 = 352;
pub(crate) const AIRPLAY_AAC_FRAMES_PER_PACKET: u32 = 1024;

pub(crate) fn next_airplay_packet_timestamp(rtp_timestamp: u32, sample_rate: u32) -> u32 {
    rtp_timestamp.wrapping_add(sample_rate / SAMPLE_RATE_44100 * AIRPLAY_ALAC_FRAMES_PER_PACKET)
}
