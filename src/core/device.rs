use crate::core::features::Features;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DeviceId(pub [u8; 6]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[non_exhaustive]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub model: String,
    pub manufacturer: Option<String>,
    pub serial_number: Option<String>,
    pub addresses: Vec<IpAddr>,
    pub port: u16,

    pub features: Features,
    pub required_sender_features: Option<Features>,
    pub public_key: Option<[u8; 32]>,
    pub source_version: Version,
    pub firmware_version: Option<String>,
    pub os_version: Option<String>,
    pub protocol_version: Option<String>,
    pub requires_password: bool,

    pub status_flags: u64,
    pub access_control: Option<u8>,

    pub pairing_identity: Option<String>,
    pub system_pairing_identity: Option<String>,
    pub bluetooth_address: Option<String>,
    pub homekit_home_id: Option<String>,

    pub group_id: Option<uuid::Uuid>,
    pub is_group_leader: bool,
    pub group_public_name: Option<String>,
    pub group_contains_discoverable_leader: bool,
    pub home_group_id: Option<String>,
    pub household_id: Option<String>,
    pub parent_group_id: Option<uuid::Uuid>,
    pub parent_group_contains_discoverable_leader: bool,
    pub tight_sync_id: Option<uuid::Uuid>,

    pub raop_port: Option<u16>,
    pub raop_encryption_types: Option<Vec<u8>>,
    pub raop_codecs: Option<Vec<u8>>,
    pub raop_transport: Option<String>,
    pub raop_metadata_types: Option<Vec<u8>>,
    pub raop_digest_auth: bool,
    pub vodka_version: Option<String>,
}

impl DeviceId {
    pub fn from_bytes(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    pub fn from_mac_string(s: &str) -> Result<Self, crate::core::error::ParseError> {
        use crate::core::error::ParseError;

        let s = s.trim();

        let bytes: Vec<u8> = if s.contains(':') {
            s.split(':')
                .map(|part| {
                    u8::from_str_radix(part, 16)
                        .map_err(|_| ParseError::InvalidHex(part.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else if s.contains('-') {
            s.split('-')
                .map(|part| {
                    u8::from_str_radix(part, 16)
                        .map_err(|_| ParseError::InvalidHex(part.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            if s.len() != 12 {
                return Err(ParseError::InvalidFormat(format!(
                    "MAC address must be 12 hex characters, got {}",
                    s.len()
                )));
            }
            (0..6)
                .map(|i| {
                    let start = i * 2;
                    u8::from_str_radix(&s[start..start + 2], 16)
                        .map_err(|_| ParseError::InvalidHex(s[start..start + 2].to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        if bytes.len() != 6 {
            return Err(ParseError::InvalidFormat(format!(
                "MAC address must have 6 bytes, got {}",
                bytes.len()
            )));
        }

        let mut arr = [0u8; 6];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    pub fn to_mac_string(&self) -> String {
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_mac_string())
    }
}

impl From<[u8; 6]> for DeviceId {
    fn from(bytes: [u8; 6]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl FromStr for DeviceId {
    type Err = crate::core::error::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_mac_string(s)
    }
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse(s: &str) -> Result<Self, crate::core::error::ParseError> {
        use crate::core::error::ParseError;

        let s = s.trim();
        if s.is_empty() {
            return Err(ParseError::InvalidFormat(
                "empty version string".to_string(),
            ));
        }

        let parts: Vec<&str> = s.split('.').collect();

        let major = parts
            .first()
            .ok_or_else(|| ParseError::InvalidFormat("missing major version".to_string()))?
            .parse::<u32>()
            .map_err(|_| {
                ParseError::InvalidValue(format!("invalid major version: {}", parts[0]))
            })?;

        let minor = parts
            .get(1)
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|_| ParseError::InvalidValue(format!("invalid minor version: {}", s)))
            })
            .transpose()?
            .unwrap_or(0);

        let patch = parts
            .get(2)
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|_| ParseError::InvalidValue(format!("invalid patch version: {}", s)))
            })
            .transpose()?
            .unwrap_or(0);

        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for Version {
    type Err = crate::core::error::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

const MIN_AIRPLAY2_VERSION: Version = Version {
    major: 354,
    minor: 54,
    patch: 6,
};

const MIN_PTP_VERSION: Version = Version {
    major: 366,
    minor: 0,
    patch: 0,
};

impl Device {
    pub fn supports_airplay2(&self) -> bool {
        self.features.supports_buffered_audio() && self.source_version >= MIN_AIRPLAY2_VERSION
    }

    pub fn supports_ptp(&self) -> bool {
        self.features.supports_ptp() && self.source_version >= MIN_PTP_VERSION
    }

    pub fn socket_addr(&self) -> Option<SocketAddr> {
        for addr in &self.addresses {
            if addr.is_ipv4() {
                return Some(SocketAddr::new(*addr, self.port));
            }
        }
        self.addresses
            .first()
            .map(|addr| SocketAddr::new(*addr, self.port))
    }

    pub fn auth_method(&self) -> crate::core::features::AuthMethod {
        self.features.auth_method()
    }

    pub fn supports_raop(&self) -> bool {
        self.raop_port.is_some() || !self.supports_airplay2()
    }

    pub fn raop_connection_port(&self) -> u16 {
        self.raop_port.unwrap_or(self.port)
    }

    pub fn supports_rsa_encryption(&self) -> bool {
        self.raop_encryption_types
            .as_ref()
            .map(|et| et.contains(&1))
            .unwrap_or(false)
    }
}
