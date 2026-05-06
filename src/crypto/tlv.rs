use crate::core::error::ParseError;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TlvType {
    Method = 0x00,
    Salt = 0x02,
    PublicKey = 0x03,
    Proof = 0x04,
    State = 0x06,
    Error = 0x07,
    Flags = 0x13,
}

#[derive(Debug, Clone, Default)]
pub struct Tlv8 {
    items: HashMap<u8, Vec<u8>>,
}

impl Tlv8 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        let mut items: HashMap<u8, Vec<u8>> = HashMap::new();
        let mut i = 0;
        let mut last_type: Option<u8> = None;

        while i < data.len() {
            if i + 2 > data.len() {
                return Err(ParseError::InvalidFormat(
                    "TLV8: truncated header".to_string(),
                ));
            }

            let typ = data[i];
            let len = data[i + 1] as usize;
            i += 2;

            if i + len > data.len() {
                return Err(ParseError::InvalidFormat(format!(
                    "TLV8: truncated value (expected {} bytes, got {})",
                    len,
                    data.len() - i
                )));
            }

            let value = &data[i..i + len];
            i += len;

            if Some(typ) == last_type {
                if let Some(existing) = items.get_mut(&typ) {
                    existing.extend_from_slice(value);
                }
            } else {
                items.entry(typ).or_default().extend_from_slice(value);
            }

            last_type = Some(typ);
        }

        Ok(Self { items })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut result = Vec::new();

        let mut types: Vec<_> = self.items.keys().collect();
        types.sort();

        for typ in types {
            let value = &self.items[typ];

            if value.is_empty() {
                result.push(*typ);
                result.push(0);
            } else {
                for chunk in value.chunks(255) {
                    result.push(*typ);
                    result.push(chunk.len() as u8);
                    result.extend_from_slice(chunk);
                }
            }
        }

        result
    }

    pub fn get(&self, typ: TlvType) -> Option<&[u8]> {
        self.items.get(&(typ as u8)).map(|v| v.as_slice())
    }

    pub fn set(&mut self, typ: TlvType, value: impl Into<Vec<u8>>) {
        self.items.insert(typ as u8, value.into());
    }

    pub fn state(&self) -> Option<u8> {
        self.get(TlvType::State).and_then(|v| v.first().copied())
    }

    pub fn error(&self) -> Option<u8> {
        self.get(TlvType::Error).and_then(|v| v.first().copied())
    }

    pub fn pair_setup_m1_with_flags() -> Self {
        let mut tlv = Self::new();
        tlv.set(TlvType::State, vec![0x01]); // State = M1
        tlv.set(TlvType::Method, vec![0x00]); // Method = PairSetup
        tlv.set(TlvType::Flags, vec![0x10, 0x00, 0x00, 0x00]);
        tlv
    }
}
