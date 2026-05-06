use crate::core::error::DiscoveryError;
use crate::core::{Device, DeviceId, Result};
use crate::discovery::parser::TxtRecordParser;
use crate::discovery::traits::{BrowseEvent, Discovery};
use crate::discovery::{AIRPLAY_SERVICE_TYPE, RAOP_SERVICE_TYPE};
use async_trait::async_trait;
use futures_core::Stream;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

pub struct ServiceBrowser {
    devices: Arc<RwLock<HashMap<DeviceId, Device>>>,
    daemon: ServiceDaemon,
    running: Arc<AtomicBool>,
}

impl ServiceBrowser {
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()
            .map_err(|e| DiscoveryError::Daemon(format!("Failed to create mDNS daemon: {}", e)))?;

        Ok(Self {
            devices: Arc::new(RwLock::new(HashMap::new())),
            daemon,
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    fn parse_service_event(service_info: &mdns_sd::ServiceInfo, is_raop: bool) -> Option<Device> {
        let name = service_info.get_fullname();
        let port = service_info.get_port();

        let addresses: Vec<IpAddr> = service_info.get_addresses().iter().copied().collect();

        if addresses.is_empty() {
            debug!("Service {} has no addresses, skipping", name);
            return None;
        }

        let txt: HashMap<String, String> = service_info
            .get_properties()
            .iter()
            .map(|prop| (prop.key().to_string(), prop.val_str().to_string()))
            .collect();

        let service_name = service_info
            .get_fullname()
            .split('.')
            .next()
            .unwrap_or(name);

        let result = if is_raop {
            TxtRecordParser::parse_raop_txt(service_name, &txt, addresses, port)
        } else {
            TxtRecordParser::parse_airplay_txt(service_name, &txt, addresses, port)
        };

        match result {
            Ok(device) => {
                debug!(
                    "Parsed device: {} ({})",
                    device.name,
                    device.id.to_mac_string()
                );
                Some(device)
            }
            Err(e) => {
                warn!("Failed to parse service {}: {}", name, e);
                None
            }
        }
    }

    fn extract_device_id_from_removal(fullname: &str, is_raop: bool) -> Option<DeviceId> {
        let service_name = fullname.split('.').next()?;

        if is_raop {
            let mac_hex = service_name.split('@').next()?;
            DeviceId::from_mac_string(mac_hex).ok()
        } else {
            None
        }
    }

    async fn handle_service_event(
        event: ServiceEvent,
        is_raop: bool,
        devices: &Arc<RwLock<HashMap<DeviceId, Device>>>,
    ) -> Option<BrowseEvent> {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                trace!("Service resolved: {}", info.get_fullname());
                if let Some(device) = Self::parse_service_event(&info, is_raop) {
                    let device_id = device.id.clone();
                    let mut devices_guard = devices.write().await;

                    let is_new = !devices_guard.contains_key(&device_id);

                    if is_new {
                        devices_guard.insert(device_id, device.clone());
                        Some(BrowseEvent::Added(device))
                    } else {
                        let existing = devices_guard.get(&device_id).unwrap();
                        let merged = if is_raop {
                            TxtRecordParser::merge_device_info(existing, &device)
                        } else {
                            TxtRecordParser::merge_device_info(&device, existing)
                        };
                        devices_guard.insert(device_id, merged.clone());
                        Some(BrowseEvent::Updated(merged))
                    }
                } else {
                    None
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                trace!("Service removed: {}", fullname);
                if let Some(device_id) = Self::extract_device_id_from_removal(&fullname, is_raop) {
                    let mut devices_guard = devices.write().await;
                    if devices_guard.remove(&device_id).is_some() {
                        Some(BrowseEvent::Removed(device_id))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            ServiceEvent::SearchStarted(_) => {
                trace!("Search started");
                None
            }
            ServiceEvent::SearchStopped(_) => {
                trace!("Search stopped");
                None
            }
            _ => None,
        }
    }
}

impl Default for ServiceBrowser {
    fn default() -> Self {
        Self::new().expect("Failed to create ServiceBrowser")
    }
}

#[async_trait]
impl Discovery for ServiceBrowser {
    async fn browse(&self) -> Result<Box<dyn Stream<Item = BrowseEvent> + Send + Unpin>> {
        self.running.store(true, Ordering::SeqCst);

        let airplay_receiver = self
            .daemon
            .browse(AIRPLAY_SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Daemon(format!("Failed to browse AirPlay: {}", e)))?;

        let raop_receiver = self
            .daemon
            .browse(RAOP_SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Daemon(format!("Failed to browse RAOP: {}", e)))?;

        let devices = Arc::clone(&self.devices);
        let running = Arc::clone(&self.running);

        let stream = async_stream::stream! {
            loop {
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                let recv_timeout = Duration::from_millis(100);

                if let Ok(event) = airplay_receiver.recv_timeout(recv_timeout)
                    && let Some(browse_event) = Self::handle_service_event(event, false, &devices).await
                {
                    yield browse_event;
                }

                if let Ok(event) = raop_receiver.recv_timeout(recv_timeout)
                    && let Some(browse_event) = Self::handle_service_event(event, true, &devices).await
                {
                    yield browse_event;
                }
            }
        };

        Ok(Box::new(Box::pin(stream)))
    }

    async fn scan(&self, timeout: Duration) -> Result<Vec<Device>> {
        self.running.store(true, Ordering::SeqCst);

        let airplay_receiver = self
            .daemon
            .browse(AIRPLAY_SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Daemon(format!("Failed to browse AirPlay: {}", e)))?;

        let raop_receiver = self
            .daemon
            .browse(RAOP_SERVICE_TYPE)
            .map_err(|e| DiscoveryError::Daemon(format!("Failed to browse RAOP: {}", e)))?;

        let devices = Arc::clone(&self.devices);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout && self.running.load(Ordering::SeqCst) {
            let remaining = timeout.saturating_sub(start.elapsed());
            let recv_timeout = remaining.min(Duration::from_millis(100));

            if let Ok(event) = airplay_receiver.recv_timeout(recv_timeout) {
                Self::handle_service_event(event, false, &devices).await;
            }

            if let Ok(event) = raop_receiver.recv_timeout(recv_timeout) {
                Self::handle_service_event(event, true, &devices).await;
            }
        }

        let _ = self.daemon.stop_browse(AIRPLAY_SERVICE_TYPE);
        let _ = self.daemon.stop_browse(RAOP_SERVICE_TYPE);

        self.running.store(false, Ordering::SeqCst);

        Ok(self.get_all_devices().await)
    }

    async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.daemon.stop_browse(AIRPLAY_SERVICE_TYPE);
        let _ = self.daemon.stop_browse(RAOP_SERVICE_TYPE);
    }

    async fn get_device(&self, id: &DeviceId) -> Option<Device> {
        self.devices.read().await.get(id).cloned()
    }

    async fn get_all_devices(&self) -> Vec<Device> {
        self.devices.read().await.values().cloned().collect()
    }
}
