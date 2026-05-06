use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct DeviceStreamStats {
    pub rtx_requested: AtomicU64,
    pub rtx_fulfilled: AtomicU64,
}

impl DeviceStreamStats {
    fn new() -> Self {
        Self {
            rtx_requested: AtomicU64::new(0),
            rtx_fulfilled: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> DeviceStatsSnapshot {
        DeviceStatsSnapshot {
            rtx_requested: self.rtx_requested.load(Ordering::Relaxed),
            rtx_fulfilled: self.rtx_fulfilled.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct StreamStats {
    pub packets_sent: AtomicU64,
    pub rtx_requested: AtomicU64,
    pub rtx_fulfilled: AtomicU64,
    device_stats: Vec<DeviceStreamStats>,
}

impl StreamStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            packets_sent: AtomicU64::new(0),
            rtx_requested: AtomicU64::new(0),
            rtx_fulfilled: AtomicU64::new(0),
            device_stats: Vec::new(),
        })
    }

    pub fn with_device_count(count: usize) -> Arc<Self> {
        let mut device_stats = Vec::with_capacity(count);
        for _ in 0..count {
            device_stats.push(DeviceStreamStats::new());
        }
        Arc::new(Self {
            packets_sent: AtomicU64::new(0),
            rtx_requested: AtomicU64::new(0),
            rtx_fulfilled: AtomicU64::new(0),
            device_stats,
        })
    }

    pub fn device(&self, index: usize) -> Option<&DeviceStreamStats> {
        self.device_stats.get(index)
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            packets_sent: self.packets_sent.load(Ordering::Relaxed),
            rtx_requested: self.rtx_requested.load(Ordering::Relaxed),
            rtx_fulfilled: self.rtx_fulfilled.load(Ordering::Relaxed),
            underruns: 0, // Populated by client from streamer counter
            devices: self.device_stats.iter().map(|d| d.snapshot()).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct DeviceStatsSnapshot {
    pub rtx_requested: u64,
    pub rtx_fulfilled: u64,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct StatsSnapshot {
    pub packets_sent: u64,
    pub rtx_requested: u64,
    pub rtx_fulfilled: u64,
    pub underruns: u64,
    pub devices: Vec<DeviceStatsSnapshot>,
}

impl StatsSnapshot {
    pub fn loss_percent(&self) -> f64 {
        if self.packets_sent == 0 {
            0.0
        } else {
            (self.rtx_requested as f64 / self.packets_sent as f64) * 100.0
        }
    }

    pub fn device_loss_percent(&self, index: usize) -> f64 {
        if self.packets_sent == 0 {
            return 0.0;
        }
        if let Some(dev) = self.devices.get(index) {
            (dev.rtx_requested as f64 / self.packets_sent as f64) * 100.0
        } else {
            0.0
        }
    }
}
