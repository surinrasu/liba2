use crate::core::{Device, DeviceId, Result};
use async_trait::async_trait;
use futures_core::Stream;
use std::time::Duration;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BrowseEvent {
    Added(Device),
    Updated(Device),
    Removed(DeviceId),
}

impl BrowseEvent {
    pub fn device(&self) -> Option<&Device> {
        match self {
            BrowseEvent::Added(d) | BrowseEvent::Updated(d) => Some(d),
            BrowseEvent::Removed(_) => None,
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        match self {
            BrowseEvent::Added(d) | BrowseEvent::Updated(d) => &d.id,
            BrowseEvent::Removed(id) => id,
        }
    }

    pub fn is_added(&self) -> bool {
        matches!(self, BrowseEvent::Added(_))
    }

    pub fn is_updated(&self) -> bool {
        matches!(self, BrowseEvent::Updated(_))
    }

    pub fn is_removed(&self) -> bool {
        matches!(self, BrowseEvent::Removed(_))
    }
}

#[async_trait]
pub trait Discovery: Send + Sync {
    async fn browse(&self) -> Result<Box<dyn Stream<Item = BrowseEvent> + Send + Unpin>>;

    async fn scan(&self, timeout: Duration) -> Result<Vec<Device>>;

    async fn stop(&self);

    async fn get_device(&self, id: &DeviceId) -> Option<Device>;

    async fn get_all_devices(&self) -> Vec<Device>;
}
