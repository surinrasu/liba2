use crate::core::error::{Error, Result};
use crate::timing::Clock;
use std::sync::Arc;
use tokio::net::UdpSocket as TokioUdpSocket;
use tokio::sync::watch;

pub const TIMING_REQUEST_PT: u8 = 82;

pub const TIMING_RESPONSE_PT: u8 = 83;

#[derive(Debug, Clone, Copy)]
pub struct NtpRequest {
    pub sequence: u16,
    pub reference_time: u64,
}

impl NtpRequest {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 32 {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                "NTP request too short".into(),
            )));
        }

        let pt = data[1] & 0x7F;
        if pt != TIMING_REQUEST_PT {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                format!("Expected PT={}, got {}", TIMING_REQUEST_PT, pt),
            )));
        }

        let sequence = u16::from_be_bytes([data[2], data[3]]);
        let reference_time = u64::from_be_bytes(data[24..32].try_into().unwrap());

        Ok(Self {
            sequence,
            reference_time,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NtpResponse {
    pub sequence: u16,
    pub reference_time: u64,
    pub receive_time: u64,
    pub send_time: u64,
}

impl NtpResponse {
    pub fn from_request(request: &NtpRequest, receive_time: u64, send_time: u64) -> Self {
        Self {
            sequence: request.sequence,
            reference_time: request.reference_time, // Echo back sender's transmit time
            receive_time,
            send_time,
        }
    }

    pub fn serialize(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        buf[0] = 0x80; // V=2
        buf[1] = TIMING_RESPONSE_PT | 0x80; // PT=83 with marker
        buf[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        buf[8..16].copy_from_slice(&self.reference_time.to_be_bytes());
        buf[16..24].copy_from_slice(&self.receive_time.to_be_bytes());
        buf[24..32].copy_from_slice(&self.send_time.to_be_bytes());
        buf
    }
}

pub struct NtpTimingServer {
    _socket: Arc<TokioUdpSocket>,
    port: u16,
    shutdown_tx: watch::Sender<bool>,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl NtpTimingServer {
    pub async fn start() -> Result<Self> {
        let socket = TokioUdpSocket::bind("0.0.0.0:0").await?;
        let port = socket.local_addr()?.port();
        let socket = Arc::new(socket);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let clock = Clock::new();

        let task_socket = socket.clone();
        let task_handle = tokio::spawn(async move {
            Self::run_loop(task_socket, clock, shutdown_rx).await;
        });

        Ok(Self {
            _socket: socket,
            port,
            shutdown_tx,
            task_handle: Some(task_handle),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn stop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }

    async fn run_loop(
        socket: Arc<TokioUdpSocket>,
        clock: Clock,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        let mut buf = [0u8; 64];
        loop {
            tokio::select! {
                result = socket.recv_from(&mut buf) => {
                    if let Ok((len, addr)) = result {
                        let recv_time = clock.now_ntp();
                        if let Ok(request) = NtpRequest::parse(&buf[..len]) {
                            let send_time = clock.now_ntp();
                            let response = NtpResponse::from_request(&request, recv_time, send_time);
                            let _ = socket.send_to(&response.serialize(), addr).await;
                            tracing::debug!("NTP timing: responded to request seq={}", request.sequence);
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
    }
}
