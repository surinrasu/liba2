use crate::core::error::ParseError;
use crate::core::{Device, DeviceId, Features, Version};
use std::collections::HashMap;
use std::net::IpAddr;

pub struct TxtRecordParser;

impl TxtRecordParser {
    pub fn parse_airplay_txt(
        name: &str,
        txt: &HashMap<String, String>,
        addresses: Vec<IpAddr>,
        port: u16,
    ) -> Result<Device, ParseError> {
        let device_id_str = txt
            .get("deviceid")
            .ok_or(ParseError::MissingField("deviceid"))?;
        let id = DeviceId::from_mac_string(device_id_str)?;

        let features = txt
            .get("features")
            .map(|f| Features::from_txt_value(f))
            .transpose()?
            .unwrap_or_default();

        let model = txt.get("model").cloned().unwrap_or_default();

        let source_version = txt
            .get("srcvers")
            .map(|v| Version::parse(v))
            .transpose()?
            .unwrap_or_default();

        let public_key = txt
            .get("pk")
            .map(|pk| Self::parse_public_key(pk))
            .transpose()?;

        let requires_password = txt
            .get("pw")
            .map(|pw| pw == "true" || pw == "1")
            .unwrap_or(false);

        let required_sender_features = txt
            .get("rsf")
            .map(|f| Features::from_txt_value(f))
            .transpose()?;

        let status_flags = txt
            .get("flags")
            .or_else(|| txt.get("sf"))
            .map(|f| Self::parse_hex_or_decimal(f))
            .unwrap_or(0);

        let access_control = txt.get("acl").and_then(|v| v.parse::<u8>().ok());

        let firmware_version = txt.get("fv").cloned();

        let os_version = txt.get("osvers").cloned();

        let protocol_version = txt.get("protovers").cloned();

        let manufacturer = txt.get("manufacturer").cloned();

        let serial_number = txt.get("serialNumber").cloned();

        let bluetooth_address = txt.get("btaddr").cloned();

        let pairing_identity = txt.get("pi").cloned();
        let system_pairing_identity = txt.get("psi").cloned();

        let homekit_home_id = txt.get("hkid").cloned();

        let group_id = txt
            .get("gid")
            .map(|gid| Self::parse_uuid_prefix(gid))
            .transpose()?;

        let is_group_leader = txt
            .get("igl")
            .map(|igl| igl == "1" || igl == "true")
            .unwrap_or(false);

        let group_public_name = txt.get("gpn").cloned();

        let group_contains_discoverable_leader = txt
            .get("gcgl")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        let home_group_id = txt.get("hgid").cloned();

        let household_id = txt.get("hmid").cloned();

        let parent_group_id = txt
            .get("pgid")
            .map(|pgid| Self::parse_uuid_prefix(pgid))
            .transpose()?;

        let parent_group_contains_discoverable_leader = txt
            .get("pgcgl")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        let tight_sync_id = txt
            .get("tsid")
            .map(|tsid| Self::parse_uuid_prefix(tsid))
            .transpose()?;

        Ok(Device {
            id,
            name: name.to_string(),
            model,
            manufacturer,
            serial_number,
            addresses,
            port,
            features,
            required_sender_features,
            public_key,
            source_version,
            firmware_version,
            os_version,
            protocol_version,
            requires_password,
            status_flags,
            access_control,
            pairing_identity,
            system_pairing_identity,
            bluetooth_address,
            homekit_home_id,
            group_id,
            is_group_leader,
            group_public_name,
            group_contains_discoverable_leader,
            home_group_id,
            household_id,
            parent_group_id,
            parent_group_contains_discoverable_leader,
            tight_sync_id,
            raop_port: None,
            raop_encryption_types: None,
            raop_codecs: None,
            raop_transport: None,
            raop_metadata_types: None,
            raop_digest_auth: false,
            vodka_version: None,
        })
    }

    pub fn parse_raop_txt(
        name: &str,
        txt: &HashMap<String, String>,
        addresses: Vec<IpAddr>,
        port: u16,
    ) -> Result<Device, ParseError> {
        let at_pos = name
            .find('@')
            .ok_or_else(|| ParseError::InvalidFormat("RAOP name must contain '@'".to_string()))?;

        let mac_hex = &name[..at_pos];
        let device_name = &name[at_pos + 1..];

        let id = DeviceId::from_mac_string(mac_hex)?;

        let features = if let Some(ft) = txt.get("ft") {
            Features::from_txt_value(ft)?
        } else if let Some(sf) = txt.get("sf") {
            Self::parse_legacy_features(sf)?
        } else {
            Features::default()
        };

        let model = txt
            .get("am")
            .or_else(|| txt.get("model"))
            .cloned()
            .unwrap_or_default();

        let source_version = txt
            .get("vs")
            .or_else(|| txt.get("vn"))
            .map(|v| Version::parse(v))
            .transpose()?
            .unwrap_or_default();

        let requires_password = txt
            .get("pw")
            .map(|pw| pw == "true" || pw == "1")
            .unwrap_or(false);

        let public_key = txt
            .get("pk")
            .map(|pk| Self::parse_public_key(pk))
            .transpose()?;

        let firmware_version = txt.get("fv").cloned();

        let os_version = txt.get("ov").cloned();

        let status_flags = txt
            .get("sf")
            .map(|f| Self::parse_hex_or_decimal(f))
            .unwrap_or(0);

        let vodka_version = txt.get("vv").cloned();

        let raop_digest_auth = txt
            .get("da")
            .map(|da| da == "true" || da == "1")
            .unwrap_or(false);

        let raop_encryption_types = txt.get("et").map(|et| {
            et.split(',')
                .filter_map(|s| s.trim().parse::<u8>().ok())
                .collect()
        });

        let raop_codecs = txt.get("cn").map(|cn| {
            cn.split(',')
                .filter_map(|s| s.trim().parse::<u8>().ok())
                .collect()
        });

        let raop_transport = txt.get("tp").cloned();

        let raop_metadata_types = txt.get("md").map(|md| {
            md.split(',')
                .filter_map(|s| s.trim().parse::<u8>().ok())
                .collect()
        });

        Ok(Device {
            id,
            name: device_name.to_string(),
            model,
            manufacturer: None,
            serial_number: None,
            addresses,
            port,
            features,
            required_sender_features: None,
            public_key,
            source_version,
            firmware_version,
            os_version,
            protocol_version: None,
            requires_password,
            status_flags,
            access_control: None,
            pairing_identity: None,
            system_pairing_identity: None,
            bluetooth_address: None,
            homekit_home_id: None,
            group_id: None,
            is_group_leader: false,
            group_public_name: None,
            group_contains_discoverable_leader: false,
            home_group_id: None,
            household_id: None,
            parent_group_id: None,
            parent_group_contains_discoverable_leader: false,
            tight_sync_id: None,
            raop_port: Some(port),
            raop_encryption_types,
            raop_codecs,
            raop_transport,
            raop_metadata_types,
            raop_digest_auth,
            vodka_version,
        })
    }

    pub fn parse_public_key(hex: &str) -> Result<[u8; 32], ParseError> {
        let hex = hex.trim();

        if hex.len() != 64 {
            return Err(ParseError::InvalidFormat(format!(
                "public key must be 64 hex characters, got {}",
                hex.len()
            )));
        }

        let bytes: Vec<u8> = (0..32)
            .map(|i| {
                let start = i * 2;
                u8::from_str_radix(&hex[start..start + 2], 16)
                    .map_err(|_| ParseError::InvalidHex(hex[start..start + 2].to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }

    fn parse_uuid_prefix(value: &str) -> Result<uuid::Uuid, ParseError> {
        let uuid = value.split('+').next().unwrap_or(value);
        uuid::Uuid::parse_str(uuid)
            .map_err(|_| ParseError::InvalidValue(format!("invalid UUID: {}", value)))
    }

    fn parse_hex_or_decimal(s: &str) -> u64 {
        let s = s.trim();
        if s.starts_with("0x") || s.starts_with("0X") {
            u64::from_str_radix(&s[2..], 16).unwrap_or(0)
        } else {
            s.parse().unwrap_or(0)
        }
    }

    pub fn parse_legacy_features(sf: &str) -> Result<Features, ParseError> {
        let sf = sf.trim();

        let value = if sf.starts_with("0x") || sf.starts_with("0X") {
            u64::from_str_radix(&sf[2..], 16).map_err(|_| ParseError::InvalidHex(sf.to_string()))?
        } else if sf.chars().all(|c| c.is_ascii_digit()) {
            sf.parse::<u64>()
                .map_err(|_| ParseError::InvalidValue(format!("invalid decimal: {}", sf)))?
        } else {
            u64::from_str_radix(sf, 16).map_err(|_| ParseError::InvalidHex(sf.to_string()))?
        };

        Ok(Features::from_raw(value))
    }

    pub fn merge_device_info(airplay: &Device, raop: &Device) -> Device {
        let mut addresses = airplay.addresses.clone();
        for addr in &raop.addresses {
            if !addresses.contains(addr) {
                addresses.push(*addr);
            }
        }

        Device {
            id: airplay.id.clone(),
            name: airplay.name.clone(),
            model: if airplay.model.is_empty() {
                raop.model.clone()
            } else {
                airplay.model.clone()
            },
            manufacturer: airplay.manufacturer.clone(),
            serial_number: airplay.serial_number.clone(),
            addresses,
            port: airplay.port,
            features: airplay.features, // Prefer AirPlay features (64-bit)
            required_sender_features: airplay.required_sender_features,
            public_key: airplay.public_key.or(raop.public_key),
            source_version: if airplay.source_version == Version::default() {
                raop.source_version
            } else {
                airplay.source_version
            },
            firmware_version: airplay
                .firmware_version
                .clone()
                .or_else(|| raop.firmware_version.clone()),
            os_version: airplay
                .os_version
                .clone()
                .or_else(|| raop.os_version.clone()),
            protocol_version: airplay.protocol_version.clone(),
            requires_password: airplay.requires_password || raop.requires_password,
            status_flags: if airplay.status_flags != 0 {
                airplay.status_flags
            } else {
                raop.status_flags
            },
            access_control: airplay.access_control,
            pairing_identity: airplay.pairing_identity.clone(),
            system_pairing_identity: airplay.system_pairing_identity.clone(),
            bluetooth_address: airplay.bluetooth_address.clone(),
            homekit_home_id: airplay.homekit_home_id.clone(),
            group_id: airplay.group_id,
            is_group_leader: airplay.is_group_leader,
            group_public_name: airplay.group_public_name.clone(),
            group_contains_discoverable_leader: airplay.group_contains_discoverable_leader,
            home_group_id: airplay.home_group_id.clone(),
            household_id: airplay.household_id.clone(),
            parent_group_id: airplay.parent_group_id,
            parent_group_contains_discoverable_leader: airplay
                .parent_group_contains_discoverable_leader,
            tight_sync_id: airplay.tight_sync_id,
            raop_port: raop.raop_port,
            raop_encryption_types: raop.raop_encryption_types.clone(),
            raop_codecs: raop.raop_codecs.clone(),
            raop_transport: raop.raop_transport.clone(),
            raop_metadata_types: raop.raop_metadata_types.clone(),
            raop_digest_auth: raop.raop_digest_auth,
            vodka_version: raop.vodka_version.clone(),
        }
    }
}
