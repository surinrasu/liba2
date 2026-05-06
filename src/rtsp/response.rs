use crate::core::error::{Result, RtspError};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RtspResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl RtspResponse {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let header_end = data
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| RtspError::InvalidResponse("missing header terminator".to_string()))?;

        let header_bytes = &data[..header_end];
        let body_start = header_end + 4;

        let header_str = std::str::from_utf8(header_bytes)
            .map_err(|_| RtspError::InvalidResponse("invalid UTF-8 in headers".to_string()))?;

        let mut lines = header_str.lines();

        let status_line = lines
            .next()
            .ok_or_else(|| RtspError::InvalidResponse("missing status line".to_string()))?;

        let (status_code, status_text) = parse_status_line(status_line)?;

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_string(), value.trim().to_string());
            }
        }

        let content_length = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);

        let body = if content_length > 0 && data.len() >= body_start + content_length {
            Some(data[body_start..body_start + content_length].to_vec())
        } else if content_length > 0 {
            return Err(RtspError::InvalidResponse(format!(
                "body too short: expected {} bytes, got {}",
                content_length,
                data.len() - body_start
            ))
            .into());
        } else {
            None
        };

        Ok(Self {
            status_code,
            status_text,
            headers,
            body,
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn cseq(&self) -> Option<u32> {
        self.header("CSeq").and_then(|v| v.parse().ok())
    }
}

fn parse_status_line(line: &str) -> Result<(u16, String)> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();

    if parts.len() < 2 {
        return Err(RtspError::InvalidResponse(format!("malformed status line: {}", line)).into());
    }

    if !parts[0].starts_with("RTSP/") {
        return Err(
            RtspError::InvalidResponse(format!("not an RTSP response: {}", parts[0])).into(),
        );
    }

    let code = parts[1]
        .parse()
        .map_err(|_| RtspError::InvalidResponse(format!("invalid status code: {}", parts[1])))?;

    let text = parts.get(2).unwrap_or(&"").to_string();

    Ok((code, text))
}
