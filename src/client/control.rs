use crate::audio::{AudioStreamer, RetransmitRequest, RtpReceiver, payload_types};
use crate::client::stats::StreamStats;
use crate::client::{CONTROL_POLL_INTERVAL, GROUP_CONTROL_SOCKET_TIMEOUT};
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct ControlListener {
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ControlListener {
    pub fn spawn_single(
        receiver: Arc<RtpReceiver>,
        streamer: AudioStreamer,
        stats: Arc<StreamStats>,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let rt_handle = tokio::runtime::Handle::current();

        let handle = match std::thread::Builder::new()
            .name("control-rtx".into())
            .spawn(move || {
                tracing::debug!("Control channel listener started");
                while !thread_shutdown.load(Ordering::Relaxed) {
                    match receiver.recv_raw_timeout(CONTROL_POLL_INTERVAL) {
                        Ok(Some((data, _addr))) => {
                            let Some(request) = parse_retransmit_request(&data) else {
                                continue;
                            };

                            stats
                                .rtx_requested
                                .fetch_add(request.count as u64, Ordering::Relaxed);

                            match rt_handle.block_on(streamer.handle_retransmit(&request)) {
                                Ok(retransmitted) => {
                                    if retransmitted > 0 {
                                        stats
                                            .rtx_fulfilled
                                            .fetch_add(retransmitted as u64, Ordering::Relaxed);
                                        tracing::debug!(
                                            "Retransmitted {} packets starting from seq {}",
                                            retransmitted,
                                            request.first_sequence
                                        );
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!("Retransmit failed: {}", error);
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::trace!("Control channel recv error: {}", error);
                        }
                    }
                }
                tracing::debug!("Control channel listener stopped");
            }) {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(%error, "failed to spawn control channel listener");
                None
            }
        };

        Self { shutdown, handle }
    }

    pub fn spawn_group(
        sockets: Vec<(usize, UdpSocket)>,
        streamer: AudioStreamer,
        stats: Arc<StreamStats>,
    ) -> Option<Self> {
        if sockets.is_empty() {
            return None;
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let rt_handle = tokio::runtime::Handle::current();

        let handle = match std::thread::Builder::new()
            .name("group-ctrl".into())
            .spawn(move || {
                for (_, socket) in &sockets {
                    if let Err(error) = socket.set_read_timeout(Some(GROUP_CONTROL_SOCKET_TIMEOUT))
                    {
                        tracing::warn!(%error, "failed to set group control socket read timeout");
                    }
                }

                let mut buf = [0u8; 2048];
                tracing::debug!("Group control listener started ({} sockets)", sockets.len());

                while !thread_shutdown.load(Ordering::Relaxed) {
                    for &(device_index, ref socket) in &sockets {
                        if thread_shutdown.load(Ordering::Relaxed) {
                            break;
                        }

                        match socket.recv_from(&mut buf) {
                            Ok((len, _)) => {
                                let Some(request) = parse_retransmit_request(&buf[..len]) else {
                                    continue;
                                };

                                stats
                                    .rtx_requested
                                    .fetch_add(request.count as u64, Ordering::Relaxed);
                                if let Some(device_stats) = stats.device(device_index) {
                                    device_stats
                                        .rtx_requested
                                        .fetch_add(request.count as u64, Ordering::Relaxed);
                                }

                                match rt_handle.block_on(
                                    streamer.handle_retransmit_for_target(device_index, &request),
                                ) {
                                    Ok(fulfilled) => {
                                        if fulfilled > 0 {
                                            stats
                                                .rtx_fulfilled
                                                .fetch_add(fulfilled as u64, Ordering::Relaxed);
                                            if let Some(device_stats) = stats.device(device_index) {
                                                device_stats
                                                    .rtx_fulfilled
                                                    .fetch_add(fulfilled as u64, Ordering::Relaxed);
                                            }
                                            tracing::debug!(
                                                "Group RTX[{}]: retransmitted {}/{} (seq {}..{})",
                                                device_index,
                                                fulfilled,
                                                request.count,
                                                request.first_sequence,
                                                request
                                                    .first_sequence
                                                    .wrapping_add(request.count.saturating_sub(1))
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            "Group RTX[{}]: retransmit failed: {}",
                                            device_index,
                                            error
                                        );
                                    }
                                }
                            }
                            Err(ref error)
                                if error.kind() == std::io::ErrorKind::WouldBlock
                                    || error.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(_) => {}
                        }
                    }
                }

                tracing::debug!("Group control listener stopped");
            }) {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(%error, "failed to spawn group control listener");
                None
            }
        };

        Some(Self { shutdown, handle })
    }

    pub async fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            match tokio::task::spawn_blocking(move || handle.join()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => tracing::warn!("control listener thread panicked during shutdown"),
                Err(error) => tracing::warn!(%error, "failed to join control listener thread"),
            }
        }
    }
}

impl Drop for ControlListener {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn parse_retransmit_request(data: &[u8]) -> Option<RetransmitRequest> {
    if data.len() < 4 {
        return None;
    }

    let payload_type = data[1] & 0x7F;
    if payload_type != payload_types::RETRANSMIT_REQUEST {
        return None;
    }

    if data.len() == 8 {
        let first_sequence = u16::from_be_bytes([data[4], data[5]]);
        let count = u16::from_be_bytes([data[6], data[7]]);
        Some(RetransmitRequest {
            first_sequence,
            count,
        })
    } else if data.len() >= 12 {
        RetransmitRequest::parse(data).ok()
    } else {
        None
    }
}
