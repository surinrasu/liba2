use crate::protocol::{AIRPLAY_ACTIVE_REMOTE, AIRPLAY_USER_AGENT};
use std::collections::HashMap;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspMethod {
    Options,
    Setup,
    Record,
    Flush,
    Teardown,
    SetParameter,
    Post, // For HTTP-style endpoints like /info
    Get,
    SetPeers,
}

#[derive(Debug, Clone)]
pub struct RtspRequest {
    pub method: RtspMethod,
    pub uri: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl RtspRequest {
    pub fn new(method: RtspMethod, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            headers: HashMap::new(),
            body: None,
        }
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn content_type_bplist(self) -> Self {
        self.header("Content-Type", "application/x-apple-binary-plist")
    }

    pub fn serialize(&self, cseq: u32) -> Vec<u8> {
        let mut out = Vec::new();

        write!(
            &mut out,
            "{} {} RTSP/1.0\r\n",
            self.method.as_str(),
            self.uri
        )
        .unwrap();

        write!(&mut out, "CSeq: {}\r\n", cseq).unwrap();

        if let Some(ref body) = self.body {
            write!(&mut out, "Content-Length: {}\r\n", body.len()).unwrap();
        }

        let mut sorted_headers: Vec<_> = self.headers.iter().collect();
        sorted_headers.sort_by(|a, b| a.0.cmp(b.0));

        for (key, value) in sorted_headers {
            write!(&mut out, "{}: {}\r\n", key, value).unwrap();
        }

        out.extend_from_slice(b"\r\n");

        if let Some(ref body) = self.body {
            out.extend_from_slice(body);
        }

        out
    }

    pub fn get_info() -> Self {
        Self::new(RtspMethod::Get, "/info").content_type_bplist()
    }

    pub fn options() -> Self {
        Self::new(RtspMethod::Options, "*")
    }

    pub fn setup(uri: impl Into<String>, body: Vec<u8>) -> Self {
        Self::new(RtspMethod::Setup, uri)
            .content_type_bplist()
            .body(body)
    }

    pub fn record(uri: impl Into<String>) -> Self {
        Self::new(RtspMethod::Record, uri).header("X-Apple-ProtocolVersion", "1")
    }

    pub fn teardown(uri: impl Into<String>) -> Self {
        Self::new(RtspMethod::Teardown, uri)
    }

    pub fn flush_with_info(uri: impl Into<String>, seq: u16, rtptime: u32) -> Self {
        Self::new(RtspMethod::Flush, uri)
            .header("RTP-Info", format!("seq={};rtptime={}", seq, rtptime))
    }

    pub fn set_parameter_text(uri: impl Into<String>, body: Vec<u8>) -> Self {
        Self::new(RtspMethod::SetParameter, uri)
            .header("Content-Type", "text/parameters")
            .body(body)
    }

    pub fn setpeers(_session_id: &str, body: Vec<u8>) -> Self {
        Self::new(RtspMethod::SetPeers, "/peer-list-changed")
            .content_type_bplist()
            .body(body)
    }

    pub fn record_with_info(uri: impl Into<String>, seq: u16, rtptime: u32) -> Self {
        Self::new(RtspMethod::Record, uri)
            .header("X-Apple-ProtocolVersion", "1")
            .header("Range", "npt=0-")
            .header("RTP-Info", format!("seq={};rtptime={}", seq, rtptime))
    }

    pub fn pair_setup(body: Vec<u8>, device_id: &str, hkp: u8) -> Self {
        let dacp_id = device_id.replace(":", "");
        Self::new(RtspMethod::Post, "/pair-setup")
            .header("Content-Type", "application/octet-stream")
            .header("X-Apple-Device-ID", device_id)
            .header("X-Apple-HKP", hkp.to_string())
            .header("DACP-ID", dacp_id)
            .header("Active-Remote", AIRPLAY_ACTIVE_REMOTE)
            .header("User-Agent", AIRPLAY_USER_AGENT)
            .body(body)
    }

    pub fn feedback(uri: impl Into<String>) -> Self {
        Self::new(RtspMethod::Post, format!("{}/feedback", uri.into())).content_type_bplist()
    }
}

impl RtspMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Options => "OPTIONS",
            Self::Setup => "SETUP",
            Self::Record => "RECORD",
            Self::Flush => "FLUSH",
            Self::Teardown => "TEARDOWN",
            Self::SetParameter => "SET_PARAMETER",
            Self::Post => "POST",
            Self::Get => "GET",
            Self::SetPeers => "SETPEERS",
        }
    }
}
