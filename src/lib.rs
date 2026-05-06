pub mod audio;
mod client;
mod core;
mod crypto;
mod discovery;
mod pairing;
mod rtsp;
mod timing;

mod protocol;

pub use audio::{LiveAudioDecoder, LiveAudioSendError, LiveFrameSender, LivePcmFrame};
pub use client::{
    AirPlayClient, ClientBuilder, DeviceStatsSnapshot, PlaybackInfo, PlaybackState, StatsSnapshot,
};
pub use core::{
    AudioCodec, AudioFormat, AuthMethod, CryptoError, Device, DeviceId, DiscoveryError, Error,
    Features, PairingError, ParseError, PtpMode, Result, RtspError, SampleRate, StreamConfig,
    StreamType, StreamingError, TimingProtocol, Version,
};
pub use discovery::{BrowseEvent, Discovery, ServiceBrowser};
pub use timing::ClockOffset;
