use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("Discovery error: {0}")]
    Discovery(#[from] DiscoveryError),

    #[error("Connection error: {0}")]
    Connection(#[from] std::io::Error),

    #[error("Pairing error: {0}")]
    Pairing(#[from] PairingError),

    #[error("RTSP error: {0}")]
    Rtsp(#[from] RtspError),

    #[error("Streaming error: {0}")]
    Streaming(#[from] StreamingError),

    #[error("Crypto error: {0}")]
    Crypto(#[from] CryptoError),

    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Device does not support required feature: {feature}")]
    UnsupportedFeature { feature: &'static str },

    #[error("Device requires MFi authentication (not implementable without Apple hardware)")]
    MfiRequired,

    #[error("Operation timed out")]
    Timeout,
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum DiscoveryError {
    #[error("mDNS daemon error: {0}")]
    Daemon(String),

    #[error("Service resolution failed: {0}")]
    Resolution(String),

    #[error("No devices found")]
    NoDevicesFound,

    #[error("Device not found: {0}")]
    DeviceNotFound(String),
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum PairingError {
    #[error("Invalid PIN")]
    InvalidPin,

    #[error("Pairing rejected by device")]
    Rejected,

    #[error("SRP verification failed")]
    SrpVerificationFailed,

    #[error("Invalid server public key")]
    InvalidServerPublicKey,

    #[error("Signature verification failed")]
    SignatureInvalid,

    #[error("Pairing state mismatch: expected {expected}, got {actual}")]
    StateMismatch { expected: u8, actual: u8 },

    #[error("TLV parsing error: {0}")]
    TlvParse(String),

    #[error("Missing required TLV type: {0}")]
    MissingTlv(u8),

    #[error("Invalid pairing state: {0}")]
    InvalidState(String),

    #[error("Protocol error: {0}")]
    Protocol(String),
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum RtspError {
    #[error("Connection refused")]
    ConnectionRefused,

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Unexpected status code: {0}")]
    UnexpectedStatus(u16),

    #[error("Missing required header: {0}")]
    MissingHeader(String),

    #[error("Plist serialization error: {0}")]
    PlistError(String),

    #[error("Session not established")]
    NoSession,

    #[error("Setup failed: {0}")]
    SetupFailed(String),
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum StreamingError {
    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Buffer underrun")]
    BufferUnderrun,

    #[error("Buffer overflow")]
    BufferOverflow,

    #[error("Timing sync lost")]
    TimingSyncLost,

    #[error("Stream interrupted")]
    Interrupted,

    #[error("Invalid audio format: {0}")]
    InvalidFormat(String),
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    Encryption(String),

    #[error("Decryption failed: {0}")]
    Decryption(String),

    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),

    #[error("Invalid key length: expected {expected}, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },

    #[error("Authentication tag mismatch")]
    AuthTagMismatch,
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ParseError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Invalid hex value: {0}")]
    InvalidHex(String),

    #[error("Invalid value: {0}")]
    InvalidValue(String),
}

pub type Result<T> = std::result::Result<T, Error>;
