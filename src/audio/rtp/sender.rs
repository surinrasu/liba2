use super::packet::{RtpHeader, RtpPacket};
use super::retransmit::{RetransmitRequest, build_retransmit_response};
use super::socket::set_socket_qos;
use crate::audio::cipher::PacketCipher;
use crate::core::error::{Error, Result};
use std::net::{SocketAddr, UdpSocket};

const PACKET_HISTORY_SIZE: usize = 512;

pub struct RtpSender {
    socket: Option<UdpSocket>,
    dest: SocketAddr,
    control_dest: Option<SocketAddr>,
    control_socket: Option<UdpSocket>,
    sequence: u16,
    ssrc: u32,
    cipher: Option<Box<dyn PacketCipher>>,
    sync_sequence: u16,
    first_sync_sent: bool,
    packet_history: Vec<Option<Vec<u8>>>,
}

impl RtpSender {
    pub fn new(dest: SocketAddr, ssrc: u32) -> Self {
        let mut packet_history = Vec::with_capacity(PACKET_HISTORY_SIZE);
        packet_history.resize_with(PACKET_HISTORY_SIZE, || None);

        Self {
            socket: None,
            dest,
            control_dest: None,
            control_socket: None,
            sequence: 0,
            ssrc,
            cipher: None,
            sync_sequence: 0,
            first_sync_sent: false,
            packet_history,
        }
    }

    pub fn set_control_dest(&mut self, dest: SocketAddr) {
        self.control_dest = Some(dest);
    }

    pub fn set_control_socket(&mut self, socket: UdpSocket) {
        self.control_socket = Some(socket);
    }

    pub fn bind(&mut self, local_port: u16) -> Result<u16> {
        let socket = UdpSocket::bind(("0.0.0.0", local_port))?;
        set_socket_qos(&socket);
        let port = socket.local_addr()?.port();
        self.socket = Some(socket);
        Ok(port)
    }

    pub fn set_cipher(&mut self, cipher: Box<dyn PacketCipher>) {
        self.cipher = Some(cipher);
    }

    pub fn reset_sync_state(&mut self) {
        self.first_sync_sent = false;
    }

    fn serialize_audio(
        &mut self,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
        marker: bool,
    ) -> Result<Vec<u8>> {
        let header =
            RtpHeader::new(payload_type, self.sequence, timestamp, self.ssrc).with_marker(marker);

        let serialized = if let Some(cipher) = &self.cipher {
            let seq_before = self.sequence;
            let encrypted = cipher.encrypt_payload(payload, timestamp, self.ssrc, self.sequence)?;

            if seq_before.is_multiple_of(500)
                && let (Some(nonce), Some(tag)) = (&encrypted.nonce, &encrypted.tag)
            {
                tracing::info!(
                    "Encrypted packet: seq={}, ts={}, ssrc={:08x}, nonce={:02x?}, tag_first4={:02x?}, payload_len={}",
                    seq_before,
                    timestamp,
                    self.ssrc,
                    nonce,
                    &tag[..4],
                    payload.len()
                );
            }

            let header_bytes = header.serialize();
            let mut out = Vec::with_capacity(
                12 + encrypted.data.len()
                    + encrypted.tag.map(|_| 16).unwrap_or(0)
                    + encrypted.nonce.map(|_| 8).unwrap_or(0),
            );
            out.extend_from_slice(&header_bytes);
            out.extend_from_slice(&encrypted.data);
            if let Some(tag) = &encrypted.tag {
                out.extend_from_slice(tag);
            }
            if let Some(nonce) = &encrypted.nonce {
                out.extend_from_slice(nonce);
            }
            out
        } else {
            let packet = RtpPacket::new(header, payload.to_vec());
            packet.serialize()
        };

        if self.sequence == 0 {
            let has_cipher = self.cipher.is_some();
            tracing::info!(
                "DIAG first audio packet: wire_len={}, payload_len={}, encrypted={}, dest={}, \
                 header_first4={:02x?}, pt={}, marker={}, ssrc={:08x}",
                serialized.len(),
                payload.len(),
                has_cipher,
                self.dest,
                &serialized[..4.min(serialized.len())],
                payload_type,
                marker,
                self.ssrc,
            );
        }

        let idx = self.sequence as usize % PACKET_HISTORY_SIZE;
        self.packet_history[idx] = Some(serialized);

        tracing::debug!(
            "Audio packet prepared: seq={}, ts={}, len={}",
            self.sequence,
            timestamp,
            payload.len()
        );
        self.sequence = self.sequence.wrapping_add(1);

        Ok(self.packet_history[idx].as_ref().unwrap().clone())
    }

    pub fn send_audio(
        &mut self,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
        marker: bool,
    ) -> Result<()> {
        let serialized = self.serialize_audio(payload_type, timestamp, payload, marker)?;

        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::Streaming(crate::core::error::StreamingError::Encoding(
                "Socket not bound".into(),
            ))
        })?;
        socket.send_to(&serialized, self.dest)?;

        Ok(())
    }

    pub fn prepare_audio(
        &mut self,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
        marker: bool,
    ) -> Result<Vec<u8>> {
        self.serialize_audio(payload_type, timestamp, payload, marker)
    }

    pub fn send_sync(&mut self, rtp_timestamp: u32, ntp_timestamp: u64) -> Result<()> {
        let dest = self.control_dest.unwrap_or(self.dest);

        if dest.port() == 0 {
            tracing::debug!("Skipping sync packet (no control port for AirPlay 2 buffered)");
            return Ok(());
        }

        if self.control_socket.is_none() && self.socket.is_none() {
            return Err(Error::Streaming(
                crate::core::error::StreamingError::Encoding("Socket not bound".into()),
            ));
        }

        let packet = match self.prepare_sync(rtp_timestamp, ntp_timestamp)? {
            Some(packet) => packet,
            None => return Ok(()),
        };

        let socket = self
            .control_socket
            .as_ref()
            .or(self.socket.as_ref())
            .ok_or_else(|| {
                Error::Streaming(crate::core::error::StreamingError::Encoding(
                    "Socket not bound".into(),
                ))
            })?;

        socket.send_to(&packet, dest)?;
        tracing::debug!(
            "Sync packet sent to {}: rtp_ts={}, ntp_ts={}",
            dest,
            rtp_timestamp,
            ntp_timestamp
        );

        Ok(())
    }

    pub fn prepare_sync(
        &mut self,
        rtp_timestamp: u32,
        ntp_timestamp: u64,
    ) -> Result<Option<Vec<u8>>> {
        let dest = self.control_dest.unwrap_or(self.dest);

        if dest.port() == 0 {
            tracing::debug!("Skipping sync packet (no control port for AirPlay 2 buffered)");
            return Ok(None);
        }

        let mut packet = [0u8; 20];

        packet[0] = if !self.first_sync_sent {
            self.first_sync_sent = true;
            0x90
        } else {
            0x80
        };
        packet[1] = 0xd4;
        packet[2..4].copy_from_slice(&self.sync_sequence.to_be_bytes());
        self.sync_sequence = self.sync_sequence.wrapping_add(1);
        packet[4..8].copy_from_slice(&rtp_timestamp.to_be_bytes());
        packet[8..16].copy_from_slice(&ntp_timestamp.to_be_bytes());
        packet[16..20].copy_from_slice(&rtp_timestamp.to_be_bytes());

        if self.sync_sequence <= 2 {
            let ntp_secs = (ntp_timestamp >> 32) as u32;
            let ntp_frac = ntp_timestamp as u32;
            tracing::info!(
                "DIAG sync #{}: dest={}, rtp_ts={}, ntp_secs={}, ntp_frac={}, first_byte=0x{:02x}, pkt={:02x?}",
                self.sync_sequence - 1,
                dest,
                rtp_timestamp,
                ntp_secs,
                ntp_frac,
                packet[0],
                &packet[..20]
            );
        }

        Ok(Some(packet.to_vec()))
    }

    pub fn prepare_ptp_sync(
        &mut self,
        current_rtp_ts: u32,
        ptp_clock_ns: u64,
        next_rtp_ts: u32,
        master_clock_id: &[u8; 8],
    ) -> Result<Option<Vec<u8>>> {
        let dest = self.control_dest.unwrap_or(self.dest);

        if dest.port() == 0 {
            tracing::debug!("Skipping PTP sync packet (no control port)");
            return Ok(None);
        }

        let mut packet = [0u8; 28];

        packet[0] = if !self.first_sync_sent {
            self.first_sync_sent = true;
            0x90
        } else {
            0x80
        };

        packet[1] = 0xD7;
        packet[2..4].copy_from_slice(&self.sync_sequence.to_be_bytes());
        self.sync_sequence = self.sync_sequence.wrapping_add(1);
        packet[4..8].copy_from_slice(&current_rtp_ts.to_be_bytes());

        let ptp_secs = (ptp_clock_ns / 1_000_000_000) as u32;
        let ptp_nanos = ptp_clock_ns % 1_000_000_000;
        let ptp_frac = ((ptp_nanos << 32) / 1_000_000_000) as u32;
        packet[8..12].copy_from_slice(&ptp_secs.to_be_bytes());
        packet[12..16].copy_from_slice(&ptp_frac.to_be_bytes());
        packet[16..20].copy_from_slice(&next_rtp_ts.to_be_bytes());
        packet[20..28].copy_from_slice(master_clock_id);

        if self.sync_sequence <= 2 {
            tracing::info!(
                "DIAG PTP sync #{}: dest={}, rtp_ts={}, ptp_secs={}, ptp_frac={}, clock_id={:02x?}",
                self.sync_sequence - 1,
                dest,
                current_rtp_ts,
                ptp_secs,
                ptp_frac,
                master_clock_id
            );
        }

        Ok(Some(packet.to_vec()))
    }

    pub fn send_ptp_sync(
        &mut self,
        current_rtp_ts: u32,
        ptp_clock_ns: u64,
        next_rtp_ts: u32,
        master_clock_id: &[u8; 8],
    ) -> Result<()> {
        let packet = match self.prepare_ptp_sync(
            current_rtp_ts,
            ptp_clock_ns,
            next_rtp_ts,
            master_clock_id,
        )? {
            Some(p) => p,
            None => return Ok(()),
        };

        let dest = self.control_dest.unwrap_or(self.dest);
        let socket = self
            .control_socket
            .as_ref()
            .or(self.socket.as_ref())
            .ok_or_else(|| {
                Error::Streaming(crate::core::error::StreamingError::Encoding(
                    "Socket not bound".into(),
                ))
            })?;

        socket.send_to(&packet, dest)?;
        tracing::debug!(
            "PTP sync packet sent to {}: rtp_ts={}",
            dest,
            current_rtp_ts
        );

        Ok(())
    }

    pub fn clone_data_socket(&self) -> Result<Option<(UdpSocket, SocketAddr)>> {
        match &self.socket {
            Some(s) => Ok(Some((s.try_clone()?, self.dest))),
            None => Ok(None),
        }
    }

    pub fn clone_control_socket(&self) -> Result<Option<(UdpSocket, SocketAddr)>> {
        match (&self.control_socket, self.control_dest) {
            (Some(s), Some(dest)) => Ok(Some((s.try_clone()?, dest))),
            _ => Ok(None),
        }
    }

    pub fn handle_retransmit(&self, request: &RetransmitRequest) -> Result<u16> {
        let socket = self
            .control_socket
            .as_ref()
            .or(self.socket.as_ref())
            .ok_or_else(|| {
                Error::Streaming(crate::core::error::StreamingError::Encoding(
                    "Socket not bound".into(),
                ))
            })?;

        let dest = self.control_dest.unwrap_or(self.dest);
        let mut retransmitted = 0u16;

        for i in 0..request.count {
            let seq = request.first_sequence.wrapping_add(i);
            let idx = seq as usize % PACKET_HISTORY_SIZE;

            if let Some(ref original) = self.packet_history[idx] {
                if original.len() >= 4 {
                    let stored_seq = u16::from_be_bytes([original[2], original[3]]);
                    if stored_seq != seq {
                        tracing::debug!(
                            "Retransmit: seq {} not in history (slot has seq {})",
                            seq,
                            stored_seq
                        );
                        continue;
                    }
                }

                let response = build_retransmit_response(original);
                socket.send_to(&response, dest)?;
                retransmitted += 1;
            } else {
                tracing::debug!("Retransmit: seq {} not in history (empty slot)", seq);
            }
        }

        if retransmitted > 0 {
            tracing::debug!(
                "Retransmitted {}/{} packets (seq {}..{})",
                retransmitted,
                request.count,
                request.first_sequence,
                request.first_sequence.wrapping_add(request.count - 1)
            );
        }

        Ok(retransmitted)
    }
}
