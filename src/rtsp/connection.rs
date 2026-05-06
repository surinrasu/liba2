use std::collections::HashMap;
use std::net::SocketAddr;

use crate::core::error::{Error as CoreError, Result, RtspError};
use crate::crypto::chacha::ControlCipher;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::rtsp::{RtspRequest, RtspResponse};

pub struct RtspConnection {
    addr: SocketAddr,
    cseq: u32,
    cipher: Option<ControlCipher>,
    stream: Option<TcpStream>,
    session_headers: HashMap<String, String>,
}

impl RtspConnection {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            cseq: 0,
            cipher: None,
            stream: None,
            session_headers: HashMap::new(),
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        let stream = TcpStream::connect(self.addr)
            .await
            .map_err(|_| RtspError::ConnectionRefused)?;
        self.stream = Some(stream);
        Ok(())
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.stream.as_ref().and_then(|s| s.local_addr().ok())
    }

    pub fn set_cipher(&mut self, cipher: ControlCipher) {
        self.cipher = Some(cipher);
    }

    pub fn add_session_header(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.session_headers.insert(key.into(), value.into());
    }

    pub async fn send(&mut self, mut request: RtspRequest) -> Result<RtspResponse> {
        if self.stream.is_none() {
            return Err(RtspError::ConnectionRefused.into());
        }

        for (key, value) in &self.session_headers {
            request = request.header(key.clone(), value.clone());
        }

        let cseq = self.next_cseq();

        let request_data = request.serialize(cseq);
        tracing::debug!(
            "RTSP -> {} {} (cseq={}, encrypted={}, body_len={})",
            request.method.as_str(),
            request.uri,
            cseq,
            self.cipher.is_some(),
            request.body.as_ref().map(|b| b.len()).unwrap_or(0)
        );

        if request.method == crate::rtsp::request::RtspMethod::Setup
            || request.method == crate::rtsp::request::RtspMethod::Record
        {
            let request_str = String::from_utf8_lossy(&request_data);
            if let Some(boundary) = request_str.find("\r\n\r\n") {
                let headers = &request_str[..boundary];
                tracing::debug!("{} request headers:\n{}", request.method.as_str(), headers);
            }
        }

        let wire_data = if let Some(ref mut cipher) = self.cipher {
            let encrypted = cipher.encrypt(&request_data)?;
            tracing::debug!(
                plaintext_len = request_data.len(),
                encrypted_len = encrypted.len(),
                "Encrypted RTSP request"
            );
            encrypted
        } else {
            request_data
        };

        tracing::debug!(wire_len = wire_data.len(), "Sending wire data");
        let stream = self.stream.as_mut().unwrap();
        stream.write_all(&wire_data).await?;
        stream.flush().await?;
        tracing::debug!("Wire data sent and flushed");

        let response_data = if self.cipher.is_some() {
            timeout(
                std::time::Duration::from_secs(10),
                self.read_encrypted_response(),
            )
            .await
            .map_err(|_| CoreError::Timeout)??
        } else {
            timeout(
                std::time::Duration::from_secs(10),
                self.read_plaintext_response(),
            )
            .await
            .map_err(|_| CoreError::Timeout)??
        };

        let response = RtspResponse::parse(&response_data)?;
        tracing::debug!(
            "RTSP <- {} {} (cseq={:?})",
            response.status_code,
            response.status_text,
            response.cseq()
        );

        if response.cseq() != Some(cseq) {
            tracing::warn!(
                "CSeq mismatch: expected {}, got {:?}",
                cseq,
                response.cseq()
            );
        }

        Ok(response)
    }

    async fn read_plaintext_response(&mut self) -> Result<Vec<u8>> {
        let stream = self.stream.as_mut().unwrap();
        let mut reader = BufReader::new(stream);
        let mut response_data = Vec::new();

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            response_data.extend_from_slice(line.as_bytes());

            if line == "\r\n" {
                break;
            }
        }

        let header_str = String::from_utf8_lossy(&response_data);
        let content_length = header_str
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                if key.trim().eq_ignore_ascii_case("Content-Length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).await?;
            response_data.extend_from_slice(&body);
        }

        Ok(response_data)
    }

    async fn read_encrypted_response(&mut self) -> Result<Vec<u8>> {
        let stream = self.stream.as_mut().unwrap();
        let mut response_data = Vec::new();

        tracing::debug!("Waiting for encrypted response...");

        loop {
            let mut len_buf = [0u8; 2];
            tracing::debug!("Reading 2-byte length prefix...");
            stream.read_exact(&mut len_buf).await?;
            let block_len = u16::from_le_bytes(len_buf);
            tracing::debug!("Received block length: {}", block_len);

            let mut cipher_block = vec![0u8; block_len as usize + 16];
            stream.read_exact(&mut cipher_block).await?;

            let plain_block = self
                .cipher
                .as_mut()
                .ok_or_else(|| RtspError::InvalidResponse("Missing cipher".to_string()))?
                .decrypt_block(&cipher_block, block_len)
                .map_err(CoreError::from)?;

            response_data.extend_from_slice(&plain_block);

            if let Some(end) = response_data.windows(4).position(|w| w == b"\r\n\r\n") {
                let header_str = String::from_utf8_lossy(&response_data[..end]);
                let content_length = header_str
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        if key.trim().eq_ignore_ascii_case("Content-Length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                let total_len = end + 4 + content_length;
                if response_data.len() >= total_len {
                    response_data.truncate(total_len);
                    return Ok(response_data);
                }
            }
        }
    }

    fn next_cseq(&mut self) -> u32 {
        self.cseq += 1;
        self.cseq
    }

    pub async fn close(&mut self) -> Result<()> {
        if let Some(stream) = self.stream.take() {
            drop(stream);
        }
        Ok(())
    }
}
