use crate::core::error::{Error, PairingError, Result};
use crate::crypto::{
    keys::SharedSecret,
    srp::{SrpChallenge, SrpClient, SrpProof},
    tlv::{Tlv8, TlvType},
};

pub struct PairSetup {
    state: PairSetupState,
    pin: String,
    srp_client: Option<SrpClient>,
    srp_proof: Option<SrpProof>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairSetupState {
    Initial,
    M1Sent,
    M2Received,
    M3Sent,
    M4Received,
    Complete,
    Failed,
}

impl PairSetup {
    pub fn new_transient_with_pin(pin: &str) -> Self {
        Self {
            state: PairSetupState::Initial,
            pin: pin.to_string(),
            srp_client: None,
            srp_proof: None,
        }
    }

    pub fn generate_m1(&mut self) -> Result<Vec<u8>> {
        if self.state != PairSetupState::Initial {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::InvalidState(
                "M1 can only be generated from Initial state".to_string(),
            )));
        }

        self.srp_client = Some(SrpClient::new(b"Pair-Setup", self.pin.as_bytes()));

        let tlv = Tlv8::pair_setup_m1_with_flags();

        self.state = PairSetupState::M1Sent;
        Ok(tlv.encode())
    }

    pub fn process_m2(&mut self, response: &[u8]) -> Result<()> {
        if self.state != PairSetupState::M1Sent {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::InvalidState(
                "M2 can only be processed after M1".to_string(),
            )));
        }

        let tlv = Tlv8::parse(response).map_err(|e| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(format!("Failed to parse M2: {}", e)))
        })?;

        if let Some(_error_code) = tlv.error() {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(
                "Server returned error in M2".to_string(),
            )));
        }

        if tlv.state() != Some(0x02) {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(
                "M2 has wrong state value".to_string(),
            )));
        }

        let salt = tlv.get(TlvType::Salt).ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol("M2 missing salt".to_string()))
        })?;

        if salt.len() != 16 {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(format!(
                "M2 salt has wrong length: {} (expected 16)",
                salt.len()
            ))));
        }

        let server_pk_raw = tlv.get(TlvType::PublicKey).ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(
                "M2 missing server public key".to_string(),
            ))
        })?;

        let server_pk = if server_pk_raw.len() < 384 {
            let mut padded = vec![0u8; 384 - server_pk_raw.len()];
            padded.extend_from_slice(server_pk_raw);
            padded
        } else if server_pk_raw.len() == 384 {
            server_pk_raw.to_vec()
        } else {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(format!(
                "M2 public key too long: {} (expected <= 384)",
                server_pk_raw.len()
            ))));
        };

        let mut salt_arr = [0u8; 16];
        salt_arr.copy_from_slice(salt);

        let challenge = SrpChallenge {
            salt: salt_arr,
            server_public_key: server_pk,
        };

        let srp_client = self.srp_client.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(
                "SRP client not initialized".to_string(),
            ))
        })?;

        let proof = srp_client.process_challenge(&challenge).map_err(|e| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(format!(
                "Failed to process SRP challenge: {}",
                e
            )))
        })?;

        self.srp_proof = Some(proof);
        self.state = PairSetupState::M2Received;
        Ok(())
    }

    pub fn generate_m3(&mut self) -> Result<Vec<u8>> {
        if self.state != PairSetupState::M2Received {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::InvalidState(
                "M3 can only be generated after processing M2".to_string(),
            )));
        }

        let srp_client = self.srp_client.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(
                "SRP client not initialized".to_string(),
            ))
        })?;

        let proof = self.srp_proof.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol("SRP proof not computed".to_string()))
        })?;

        let mut tlv = Tlv8::new();
        tlv.set(TlvType::State, vec![0x03]);
        tlv.set(TlvType::PublicKey, srp_client.public_key());
        tlv.set(TlvType::Proof, proof.client_proof.clone());

        self.state = PairSetupState::M3Sent;
        Ok(tlv.encode())
    }

    pub fn process_m4(&mut self, response: &[u8]) -> Result<()> {
        if self.state != PairSetupState::M3Sent {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::InvalidState(
                "M4 can only be processed after M3".to_string(),
            )));
        }

        let tlv = Tlv8::parse(response).map_err(|e| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(format!("Failed to parse M4: {}", e)))
        })?;

        if let Some(_error_code) = tlv.error() {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(
                "Server returned error in M4 (wrong PIN?)".to_string(),
            )));
        }

        if tlv.state() != Some(0x04) {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(
                "M4 has wrong state value".to_string(),
            )));
        }

        let server_proof = tlv.get(TlvType::Proof).ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(
                "M4 missing server proof".to_string(),
            ))
        })?;

        let srp_client = self.srp_client.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol(
                "SRP client not initialized".to_string(),
            ))
        })?;

        let proof = self.srp_proof.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol("SRP proof not computed".to_string()))
        })?;

        if !srp_client.verify_server_proof(server_proof, &proof.expected_server_proof) {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::Protocol(
                "Server proof verification failed".to_string(),
            )));
        }

        self.state = PairSetupState::M4Received;
        Ok(())
    }

    pub fn complete_transient(&mut self) -> Result<SharedSecret> {
        if self.state != PairSetupState::M4Received {
            self.state = PairSetupState::Failed;
            return Err(Error::Pairing(PairingError::InvalidState(
                "complete_transient() can only be called after processing M4".to_string(),
            )));
        }

        let proof = self.srp_proof.as_ref().ok_or_else(|| {
            self.state = PairSetupState::Failed;
            Error::Pairing(PairingError::Protocol("SRP proof not computed".to_string()))
        })?;

        self.state = PairSetupState::Complete;
        Ok(SharedSecret::new(proof.shared_secret.clone()))
    }
}
