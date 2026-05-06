mod api;
mod builder;
mod connection;
mod control;
mod group;
mod handshake;
mod identity;
mod playback;
mod stats;

use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;
use tracing::debug;

pub use api::AirPlayClient;
pub use builder::ClientBuilder;
pub(crate) use connection::Connection;
pub use playback::{PlaybackInfo, PlaybackState};
pub use stats::{DeviceStatsSnapshot, StatsSnapshot};

pub(crate) const DEFAULT_RENDER_DELAY_MS: u32 = 200;
pub(crate) const DEFAULT_TRANSIENT_PIN: &str = "3939";
pub(crate) const LIVE_FRAME_CHANNEL_CAPACITY: usize = 16;

pub(crate) const PLAYBACK_COMPLETION_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(5);
pub(crate) const GROUP_CONTROL_SOCKET_TIMEOUT: Duration = Duration::from_millis(1);
pub(crate) const RECORD_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const FEEDBACK_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) const BMCA_YIELD_PRIORITY: u8 = 250;
pub(crate) const BMCA_CLOCK_ID_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn select_best_address(addresses: &[IpAddr]) -> Option<&IpAddr> {
    if let Some(addr) = addresses.iter().find(|addr| addr.is_ipv4()) {
        debug!("Selected IPv4 address: {}", addr);
        return Some(addr);
    }

    if let Some(addr) = addresses.iter().find(|addr| match addr {
        IpAddr::V6(v6) => !v6.is_loopback() && !is_link_local_v6(v6),
        _ => false,
    }) {
        debug!("Selected global IPv6 address: {}", addr);
        return Some(addr);
    }

    let addr = addresses.first();
    if let Some(addr) = addr {
        debug!("Selected fallback address: {}", addr);
    }
    addr
}

fn is_link_local_v6(addr: &Ipv6Addr) -> bool {
    let segments = addr.segments();
    (segments[0] & 0xffc0) == 0xfe80
}
