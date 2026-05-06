use crate::core::error::CryptoError;

use hkdf::Hkdf;
use sha2::Sha512;

pub fn derive_key_32(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32], CryptoError> {
    let hk = Hkdf::<Sha512>::new(Some(salt), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .map_err(|_| CryptoError::KeyDerivation("HKDF expand failed".to_string()))?;
    Ok(okm)
}

pub mod constants {
    pub const CONTROL_SALT: &[u8] = b"Control-Salt";
    pub const CONTROL_WRITE_KEY_INFO: &[u8] = b"Control-Write-Encryption-Key";
    pub const CONTROL_READ_KEY_INFO: &[u8] = b"Control-Read-Encryption-Key";
}

pub fn derive_control_write_key(shared_secret: &[u8]) -> Result<[u8; 32], CryptoError> {
    derive_key_32(
        shared_secret,
        constants::CONTROL_SALT,
        constants::CONTROL_WRITE_KEY_INFO,
    )
}

pub fn derive_control_read_key(shared_secret: &[u8]) -> Result<[u8; 32], CryptoError> {
    derive_key_32(
        shared_secret,
        constants::CONTROL_SALT,
        constants::CONTROL_READ_KEY_INFO,
    )
}
