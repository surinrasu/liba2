use crate::core::error::Result;
use crate::crypto::chacha::AudioCipher;

pub struct EncryptedPayload {
    pub data: Vec<u8>,
    pub tag: Option<[u8; 16]>,
    pub nonce: Option<[u8; 8]>,
}

pub trait PacketCipher: Send {
    fn encrypt_payload(
        &self,
        payload: &[u8],
        timestamp: u32,
        ssrc: u32,
        sequence: u16,
    ) -> Result<EncryptedPayload>;
}

pub struct ChaChaPacketCipher {
    cipher: AudioCipher,
}

impl ChaChaPacketCipher {
    pub fn new(cipher: AudioCipher) -> Self {
        Self { cipher }
    }
}

impl PacketCipher for ChaChaPacketCipher {
    fn encrypt_payload(
        &self,
        payload: &[u8],
        timestamp: u32,
        ssrc: u32,
        sequence: u16,
    ) -> Result<EncryptedPayload> {
        let (ciphertext, nonce, tag) = self
            .cipher
            .encrypt_with_seq(payload, timestamp, ssrc, sequence)
            .map_err(crate::core::error::Error::Crypto)?;

        Ok(EncryptedPayload {
            data: ciphertext,
            tag: Some(tag),
            nonce: Some(nonce),
        })
    }
}
