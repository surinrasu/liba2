use crate::core::error::{Error, Result};

pub const PTP_EVENT_PORT: u16 = 319;
pub const PTP_GENERAL_PORT: u16 = 320;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PtpMessageType {
    Sync = 0x00,
    DelayReq = 0x01,
    FollowUp = 0x08,
    DelayResp = 0x09,
    Announce = 0x0B,
    Signaling = 0x0C,
}

impl PtpMessageType {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b & 0x0F {
            0x00 => Some(Self::Sync),
            0x01 => Some(Self::DelayReq),
            0x08 => Some(Self::FollowUp),
            0x09 => Some(Self::DelayResp),
            0x0B => Some(Self::Announce),
            0x0C => Some(Self::Signaling),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PtpTimestamp {
    pub seconds: u64,
    pub nanoseconds: u32,
}

impl PtpTimestamp {
    pub fn now() -> Self {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self::from_nanos(ns)
    }

    pub fn from_nanos(nanos: u128) -> Self {
        Self {
            seconds: (nanos / 1_000_000_000) as u64,
            nanoseconds: (nanos % 1_000_000_000) as u32,
        }
    }

    pub fn to_nanos(self) -> u128 {
        (self.seconds as u128) * 1_000_000_000 + (self.nanoseconds as u128)
    }

    pub fn serialize(&self) -> [u8; 10] {
        let mut buf = [0u8; 10];
        let sec_bytes = self.seconds.to_be_bytes();
        buf[0..6].copy_from_slice(&sec_bytes[2..8]);
        buf[6..10].copy_from_slice(&self.nanoseconds.to_be_bytes());
        buf
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 10 {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                "PTP timestamp too short".into(),
            )));
        }

        let mut sec_bytes = [0u8; 8];
        sec_bytes[2..8].copy_from_slice(&data[0..6]);
        let seconds = u64::from_be_bytes(sec_bytes);
        let nanoseconds = u32::from_be_bytes(data[6..10].try_into().unwrap());

        Ok(Self {
            seconds,
            nanoseconds,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PtpHeader {
    pub message_type: PtpMessageType,
    pub version: u8,
    pub message_length: u16,
    pub domain_number: u8,
    pub flags: u16,
    pub correction_field: i64,
    pub source_port_identity: [u8; 10],
    pub sequence_id: u16,
    pub control_field: u8,
    pub log_message_interval: i8,
}

impl PtpHeader {
    pub fn new(message_type: PtpMessageType, sequence_id: u16) -> Self {
        Self {
            message_type,
            version: 2,
            message_length: 44,
            domain_number: 0,
            flags: 0,
            correction_field: 0,
            source_port_identity: [0; 10],
            sequence_id,
            control_field: match message_type {
                PtpMessageType::Sync => 0,
                PtpMessageType::DelayReq => 1,
                PtpMessageType::FollowUp => 2,
                PtpMessageType::DelayResp => 3,
                PtpMessageType::Announce => 5,
                PtpMessageType::Signaling => 5,
            },
            log_message_interval: 0,
        }
    }

    pub fn serialize(&self) -> [u8; 34] {
        let mut buf = [0u8; 34];

        buf[0] = self.message_type as u8;
        buf[1] = self.version;
        buf[2..4].copy_from_slice(&self.message_length.to_be_bytes());
        buf[4] = self.domain_number;
        buf[5] = 0;
        buf[6..8].copy_from_slice(&self.flags.to_be_bytes());
        buf[8..16].copy_from_slice(&self.correction_field.to_be_bytes());
        buf[16..20].fill(0);
        buf[20..30].copy_from_slice(&self.source_port_identity);
        buf[30..32].copy_from_slice(&self.sequence_id.to_be_bytes());
        buf[32] = self.control_field;
        buf[33] = self.log_message_interval as u8;

        buf
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 34 {
            return Err(Error::Parse(crate::core::error::ParseError::InvalidFormat(
                "PTP header too short".into(),
            )));
        }

        let message_type = PtpMessageType::from_byte(data[0]).ok_or_else(|| {
            Error::Parse(crate::core::error::ParseError::InvalidFormat(format!(
                "Unknown PTP message type: {}",
                data[0] & 0x0F
            )))
        })?;

        Ok(Self {
            message_type,
            version: data[1] & 0x0F,
            message_length: u16::from_be_bytes([data[2], data[3]]),
            domain_number: data[4],
            flags: u16::from_be_bytes([data[6], data[7]]),
            correction_field: i64::from_be_bytes(data[8..16].try_into().unwrap()),
            source_port_identity: data[20..30].try_into().unwrap(),
            sequence_id: u16::from_be_bytes([data[30], data[31]]),
            control_field: data[32],
            log_message_interval: data[33] as i8,
        })
    }
}
