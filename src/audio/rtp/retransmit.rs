use super::payload_types;
use crate::core::error::{Error, Result};

#[derive(Debug, Clone, Copy)]
pub struct RetransmitRequest {
    pub first_sequence: u16,
    pub count: u16,
}

impl RetransmitRequest {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 12 {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                "Retransmit request too short".into(),
            )));
        }

        let payload_type = data[1] & 0x7F;
        if payload_type != payload_types::RETRANSMIT_REQUEST {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                format!("Expected payload type 85, got {}", payload_type),
            )));
        }

        let first_sequence = u16::from_be_bytes([data[8], data[9]]);
        let count = u16::from_be_bytes([data[10], data[11]]);

        Ok(Self {
            first_sequence,
            count,
        })
    }
}

pub fn build_retransmit_response(original_packet: &[u8]) -> Vec<u8> {
    let mut response = Vec::with_capacity(4 + original_packet.len());

    response.push(0x80);
    response.push(0x80 | payload_types::RETRANSMIT_RESPONSE);

    if original_packet.len() >= 4 {
        response.push(original_packet[2]);
        response.push(original_packet[3]);
    } else {
        response.push(0);
        response.push(0);
    }

    response.extend_from_slice(original_packet);

    response
}
