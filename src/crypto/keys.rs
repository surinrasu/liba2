use crate::core::error::CryptoError;
use crate::crypto::hkdf;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SharedSecret(pub Vec<u8>);

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey(pub [u8; 32]);

#[derive(ZeroizeOnDrop)]
pub struct SessionKeys {
    pub write_key: EncryptionKey,
    pub read_key: EncryptionKey,
}

impl SharedSecret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl EncryptionKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl SessionKeys {
    pub fn derive_control_keys(shared_secret: &SharedSecret) -> Result<Self, CryptoError> {
        let write_key = hkdf::derive_control_write_key(shared_secret.as_bytes())?;
        let read_key = hkdf::derive_control_read_key(shared_secret.as_bytes())?;

        Ok(Self {
            write_key: EncryptionKey(write_key),
            read_key: EncryptionKey(read_key),
        })
    }
}
