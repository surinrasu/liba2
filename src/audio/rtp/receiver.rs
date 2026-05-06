use super::socket::set_socket_qos;
use crate::core::error::{Error, Result};
use std::net::UdpSocket;
use std::time::Duration;

pub struct RtpReceiver {
    socket: Option<UdpSocket>,
}

impl RtpReceiver {
    pub fn new() -> Self {
        Self { socket: None }
    }

    pub fn bind(&mut self, local_port: u16) -> Result<u16> {
        let socket = UdpSocket::bind(("0.0.0.0", local_port))?;
        set_socket_qos(&socket);
        socket.set_read_timeout(Some(Duration::from_secs(1)))?;
        let port = socket.local_addr()?.port();
        self.socket = Some(socket);
        Ok(port)
    }

    pub fn recv_raw_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<(Vec<u8>, std::net::SocketAddr)>> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Streaming(crate::core::error::StreamingError::Encoding(
                "Socket not bound".into(),
            ))
        })?;

        socket.set_read_timeout(Some(timeout))?;

        let mut buf = [0u8; 2048];
        match socket.recv_from(&mut buf) {
            Ok((len, addr)) => Ok(Some((buf[..len].to_vec(), addr))),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn try_clone_socket(&self) -> Result<Option<UdpSocket>> {
        match &self.socket {
            Some(s) => Ok(Some(s.try_clone()?)),
            None => Ok(None),
        }
    }
}

impl Default for RtpReceiver {
    fn default() -> Self {
        Self::new()
    }
}
