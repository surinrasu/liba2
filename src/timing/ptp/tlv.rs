#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TlvType {
    OrganizationExtension = 0x0003,
}

pub mod org_id {
    pub const GPTP: [u8; 3] = [0x00, 0x80, 0xC2];
    pub const APPLE: [u8; 3] = [0x00, 0x0D, 0x93];
}

pub mod gptp_subtype {
    pub const FOLLOW_UP_INFO: [u8; 3] = [0x00, 0x00, 0x01];
    pub const MESSAGE_INTERVAL_REQUEST: [u8; 3] = [0x00, 0x00, 0x02];
}

pub mod apple_subtype {
    pub const TYPE_01: [u8; 3] = [0x00, 0x00, 0x01];
    pub const TYPE_05: [u8; 3] = [0x00, 0x00, 0x05];
}

#[derive(Debug, Clone)]
pub struct TlvHeader {
    pub tlv_type: u16,
    pub length: u16,
}

impl TlvHeader {
    pub fn serialize(&self) -> [u8; 4] {
        let mut buf = [0u8; 4];
        buf[0..2].copy_from_slice(&self.tlv_type.to_be_bytes());
        buf[2..4].copy_from_slice(&self.length.to_be_bytes());
        buf
    }
}

#[derive(Debug, Clone)]
pub struct FollowUpInformationTlv {
    pub organization_id: [u8; 3],
    pub organization_subtype: [u8; 3],
    pub cumulative_scaled_rate_offset: i32,
    pub gm_time_base_indicator: u16,
    pub last_gm_phase_change: [u8; 12],
    pub scaled_last_gm_freq_change: i32,
}

impl FollowUpInformationTlv {
    pub fn new() -> Self {
        Self {
            organization_id: org_id::GPTP,
            organization_subtype: gptp_subtype::FOLLOW_UP_INFO,
            cumulative_scaled_rate_offset: 0,
            gm_time_base_indicator: 0,
            last_gm_phase_change: [0; 12],
            scaled_last_gm_freq_change: 0,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32);

        let header = TlvHeader {
            tlv_type: TlvType::OrganizationExtension as u16,
            length: 28,
        };
        buf.extend_from_slice(&header.serialize());

        buf.extend_from_slice(&self.organization_id);
        buf.extend_from_slice(&self.organization_subtype);
        buf.extend_from_slice(&self.cumulative_scaled_rate_offset.to_be_bytes());
        buf.extend_from_slice(&self.gm_time_base_indicator.to_be_bytes());
        buf.extend_from_slice(&self.last_gm_phase_change);
        buf.extend_from_slice(&self.scaled_last_gm_freq_change.to_be_bytes());

        buf
    }
}

#[derive(Debug, Clone)]
pub struct MessageIntervalRequestTlv {
    pub organization_id: [u8; 3],
    pub organization_subtype: [u8; 3],
    pub link_delay_interval: i8,
    pub time_sync_interval: i8,
    pub announce_interval: i8,
    pub flags: u8,
}

impl MessageIntervalRequestTlv {
    pub fn new(sync_interval: i8, announce_interval: i8) -> Self {
        Self {
            organization_id: org_id::GPTP,
            organization_subtype: gptp_subtype::MESSAGE_INTERVAL_REQUEST,
            link_delay_interval: sync_interval,
            time_sync_interval: sync_interval,
            announce_interval,
            flags: 0x02,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(14);

        let header = TlvHeader {
            tlv_type: TlvType::OrganizationExtension as u16,
            length: 10,
        };
        buf.extend_from_slice(&header.serialize());

        buf.extend_from_slice(&self.organization_id);
        buf.extend_from_slice(&self.organization_subtype);
        buf.push(self.link_delay_interval as u8);
        buf.push(self.time_sync_interval as u8);
        buf.push(self.announce_interval as u8);
        buf.push(self.flags);

        buf
    }
}

#[derive(Debug, Clone)]
pub struct AppleTlv {
    pub organization_id: [u8; 3],
    pub organization_subtype: [u8; 3],
    pub payload: Vec<u8>,
}

impl AppleTlv {
    pub fn new(subtype: [u8; 3], payload: Vec<u8>) -> Self {
        Self {
            organization_id: org_id::APPLE,
            organization_subtype: subtype,
            payload,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        let header = TlvHeader {
            tlv_type: TlvType::OrganizationExtension as u16,
            length: (6 + self.payload.len()) as u16,
        };
        buf.extend_from_slice(&header.serialize());

        buf.extend_from_slice(&self.organization_id);
        buf.extend_from_slice(&self.organization_subtype);
        buf.extend_from_slice(&self.payload);

        buf
    }
}
