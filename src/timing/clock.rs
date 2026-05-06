use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct ClockOffset {
    pub offset_ns: i64,
    pub error_ns: u64,
    pub rtt_ns: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Clock;

impl Clock {
    pub fn new() -> Self {
        Self
    }

    pub fn now_wall_ns(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos() as u64
    }

    pub fn now_ntp(&self) -> u64 {
        unix_to_ntp(self.now_wall_ns())
    }

    pub fn apply_offset(&self, local_ns: u64, offset: &ClockOffset) -> u64 {
        (local_ns as i64 + offset.offset_ns) as u64
    }
}

pub fn unix_to_ntp(unix_ns: u64) -> u64 {
    let unix_secs = unix_ns / 1_000_000_000;
    let frac_ns = unix_ns % 1_000_000_000;

    let ntp_secs = unix_secs + NTP_EPOCH_OFFSET;
    let ntp_frac = ((frac_ns as u128) << 32) / 1_000_000_000;

    (ntp_secs << 32) | (ntp_frac as u64)
}
