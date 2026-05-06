use crate::core::error::{Result, RtspError};
use serde::{Deserialize, Serialize};

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    plist::to_writer_binary(std::io::Cursor::new(&mut buf), value)
        .map_err(|e| RtspError::PlistError(e.to_string()))?;
    Ok(buf)
}

pub fn decode<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T> {
    plist::from_bytes(data).map_err(|e| RtspError::PlistError(e.to_string()).into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingPeerInfo {
    #[serde(rename = "Addresses")]
    pub addresses: Vec<String>,
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "SupportsClockPortMatchingOverride", default)]
    pub supports_clock_port_matching_override: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupPhase1Request {
    #[serde(rename = "deviceID")]
    pub device_id: String,
    #[serde(rename = "sessionUUID")]
    pub session_uuid: String,
    #[serde(rename = "timingPort")]
    pub timing_port: u16,
    #[serde(rename = "timingProtocol")]
    pub timing_protocol: String,
    #[serde(rename = "timingPeerInfo", skip_serializing_if = "Option::is_none")]
    pub timing_peer_info: Option<TimingPeerInfo>,
    #[serde(rename = "timingPeerList", skip_serializing_if = "Option::is_none")]
    pub timing_peer_list: Option<Vec<TimingPeerInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupPhase1Response {
    #[serde(rename = "eventPort", default)]
    pub event_port: u16,
    #[serde(rename = "timingPort", default)]
    pub timing_port: u16,
    #[serde(rename = "timingPeerInfo")]
    pub timing_peer_info: Option<TimingPeerInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDef {
    #[serde(rename = "type")]
    pub stream_type: u32,
    #[serde(rename = "audioFormat")]
    pub audio_format: u32,
    #[serde(rename = "audioMode")]
    pub audio_mode: String,
    #[serde(rename = "sr")]
    pub sample_rate: u32,
    pub ct: u8,
    #[serde(rename = "controlPort")]
    pub control_port: u16,
    #[serde(rename = "isMedia")]
    pub is_media: bool,
    #[serde(rename = "latencyMin")]
    pub latency_min: u32,
    #[serde(rename = "latencyMax")]
    pub latency_max: u32,
    #[serde(with = "serde_bytes")]
    pub shk: Vec<u8>,
    #[serde(with = "serde_bytes", skip_serializing_if = "Option::is_none")]
    pub asc: Option<Vec<u8>>,
    pub spf: u32,
    #[serde(rename = "supportsDynamicStreamID")]
    pub supports_dynamic_stream_id: bool,
    #[serde(rename = "streamConnectionID")]
    pub stream_connection_id: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupPhase2Request {
    pub streams: Vec<StreamDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    #[serde(rename = "type")]
    pub stream_type: u32,
    #[serde(rename = "dataPort")]
    pub data_port: u16,
    #[serde(rename = "controlPort")]
    pub control_port: u16,
    #[serde(rename = "streamID", default)]
    pub stream_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupPhase2Response {
    pub streams: Vec<StreamResponse>,
}
