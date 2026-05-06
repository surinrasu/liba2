use crate::core::error::CryptoError;
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use zeroize::ZeroizeOnDrop;

#[derive(ZeroizeOnDrop)]
pub struct ControlCipher {
    write_key: [u8; 32],
    read_key: [u8; 32],
    #[zeroize(skip)]
    write_cipher: ChaCha20Poly1305,
    #[zeroize(skip)]
    read_cipher: ChaCha20Poly1305,
    #[zeroize(skip)]
    encrypt_counter: u64,
    #[zeroize(skip)]
    decrypt_counter: u64,
}

#[derive(ZeroizeOnDrop)]
pub struct AudioCipher {
    key: [u8; 32],
    #[zeroize(skip)]
    cipher: ChaCha20Poly1305,
}

impl ControlCipher {
    pub fn new(write_key: [u8; 32], read_key: [u8; 32]) -> Self {
        let write_cipher = ChaCha20Poly1305::new(&write_key.into());
        let read_cipher = ChaCha20Poly1305::new(&read_key.into());
        Self {
            write_key,
            read_key,
            write_cipher,
            read_cipher,
            encrypt_counter: 0,
            decrypt_counter: 0,
        }
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        const MAX_BLOCK: usize = 0x400;
        if plaintext.is_empty() {
            return Err(CryptoError::Encryption("Empty plaintext".to_string()));
        }

        let mut out = Vec::with_capacity(plaintext.len() + (plaintext.len() / MAX_BLOCK + 1) * 18);
        let mut offset = 0;
        while offset < plaintext.len() {
            let remaining = plaintext.len() - offset;
            let block_len = remaining.min(MAX_BLOCK) as u16;
            let block = &plaintext[offset..offset + block_len as usize];
            let aad = block_len.to_le_bytes();

            let nonce = build_nonce_from_counter(self.encrypt_counter);
            let nonce = Nonce::from_slice(&nonce);
            let payload = Payload {
                msg: block,
                aad: &aad,
            };

            let ciphertext_with_tag = self
                .write_cipher
                .encrypt(nonce, payload)
                .map_err(|e| CryptoError::Encryption(format!("Encryption failed: {}", e)))?;

            out.extend_from_slice(&aad);
            out.extend_from_slice(&ciphertext_with_tag);

            self.encrypt_counter += 1;
            offset += block_len as usize;
        }

        Ok(out)
    }

    pub fn decrypt_block(
        &mut self,
        ciphertext_with_tag: &[u8],
        block_len: u16,
    ) -> Result<Vec<u8>, CryptoError> {
        if ciphertext_with_tag.len() < block_len as usize + 16 {
            return Err(CryptoError::Decryption(
                "Ciphertext block too short".to_string(),
            ));
        }

        let aad = block_len.to_le_bytes();
        let nonce = build_nonce_from_counter(self.decrypt_counter);
        let nonce = Nonce::from_slice(&nonce);
        let payload = Payload {
            msg: ciphertext_with_tag,
            aad: &aad,
        };

        let plaintext = self
            .read_cipher
            .decrypt(nonce, payload)
            .map_err(|_| CryptoError::Decryption("Decryption/authentication failed".to_string()))?;

        self.decrypt_counter += 1;
        Ok(plaintext)
    }
}

impl AudioCipher {
    pub fn new(key: [u8; 32]) -> Self {
        let cipher = ChaCha20Poly1305::new(&key.into());
        Self { key, cipher }
    }

    pub fn encrypt_with_seq(
        &self,
        audio_data: &[u8],
        rtp_timestamp: u32,
        ssrc: u32,
        seqnum: u16,
    ) -> Result<AudioEncryptResult, CryptoError> {
        let mut nonce_12 = [0u8; 12];
        nonce_12[4..6].copy_from_slice(&seqnum.to_le_bytes());
        let nonce = Nonce::from_slice(&nonce_12);

        let aad = build_aad(rtp_timestamp, ssrc);

        let payload = Payload {
            msg: audio_data,
            aad: &aad,
        };

        let mut ciphertext_with_tag = self
            .cipher
            .encrypt(nonce, payload)
            .map_err(|e| CryptoError::Encryption(format!("Encryption failed: {}", e)))?;

        let tag_start = ciphertext_with_tag.len() - 16;
        let tag = ciphertext_with_tag.split_off(tag_start);
        let mut tag_array = [0u8; 16];
        tag_array.copy_from_slice(&tag);

        let mut nonce_8 = [0u8; 8];
        nonce_8.copy_from_slice(&nonce_12[4..12]);

        Ok((ciphertext_with_tag, nonce_8, tag_array))
    }
}

type AudioEncryptResult = (Vec<u8>, [u8; 8], [u8; 16]);

fn build_nonce_from_counter(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..12].copy_from_slice(&counter.to_le_bytes());
    nonce
}

fn build_aad(rtp_timestamp: u32, ssrc: u32) -> [u8; 8] {
    let mut aad = [0u8; 8];
    aad[0..4].copy_from_slice(&rtp_timestamp.to_be_bytes());
    aad[4..8].copy_from_slice(&ssrc.to_be_bytes());
    aad
}
