use crate::core::error::{Error, PairingError, Result};
use crate::crypto::keys::SessionKeys;
use crate::pairing::PairSetup;

pub struct PairingSession {
    transient_setup: Option<PairSetup>,
    transient_state: TransientStage,
    session_keys: Option<SessionKeys>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransientStage {
    NotStarted,
    WaitingM2,
    WaitingM4,
    Complete,
}

impl PairingSession {
    pub fn new() -> Self {
        Self {
            transient_setup: None,
            transient_state: TransientStage::NotStarted,
            session_keys: None,
        }
    }

    pub fn start_transient_pairing_with_pin(&mut self, pin: &str) -> Result<Vec<u8>> {
        if self.transient_setup.is_some() {
            return Err(Error::Pairing(PairingError::InvalidState(
                "Transient pairing already started".to_string(),
            )));
        }

        let mut transient = PairSetup::new_transient_with_pin(pin);
        let m1 = transient.generate_m1()?;

        self.transient_setup = Some(transient);
        self.transient_state = TransientStage::WaitingM2;

        Ok(m1)
    }

    pub fn continue_transient_pairing(&mut self, response: &[u8]) -> Result<Option<Vec<u8>>> {
        let transient = self.transient_setup.as_mut().ok_or_else(|| {
            Error::Pairing(PairingError::InvalidState(
                "Transient pairing not started".to_string(),
            ))
        })?;

        match self.transient_state {
            TransientStage::WaitingM2 => {
                transient.process_m2(response)?;
                let m3 = transient.generate_m3()?;
                self.transient_state = TransientStage::WaitingM4;
                Ok(Some(m3))
            }
            TransientStage::WaitingM4 => {
                transient.process_m4(response)?;
                let shared_secret = transient.complete_transient()?;
                let keys = SessionKeys::derive_control_keys(&shared_secret)?;

                self.session_keys = Some(keys);
                self.transient_state = TransientStage::Complete;

                Ok(None)
            }
            _ => Err(Error::Pairing(PairingError::InvalidState(
                "Invalid transient state for continue".to_string(),
            ))),
        }
    }

    pub fn take_session_keys(&mut self) -> Option<SessionKeys> {
        self.session_keys.take()
    }
}
