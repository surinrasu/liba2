use crate::client::Connection;
use crate::client::identity::generate_device_id;
use crate::client::select_best_address;
use crate::core::error::RtspError;
use crate::core::{Device, StreamConfig, error::Result};
use crate::crypto::chacha::ControlCipher;
use crate::pairing::PairingSession;
use crate::protocol::{
    AIRPLAY_ACTIVE_REMOTE, AIRPLAY_CLIENT_NAME, AIRPLAY_USER_AGENT, HKP_TRANSIENT,
};
use crate::rtsp::{RtspConnection, RtspRequest, RtspSession};
use std::net::SocketAddr;
use tracing::{debug, info, warn};

pub(crate) async fn connect_with_pin(
    device: Device,
    config: StreamConfig,
    pin: &str,
) -> Result<Connection> {
    let client_device_id = generate_device_id();
    let (mut rtsp, mut session) =
        open_connected_session(&device, &config, &client_device_id, None).await?;

    let mut pairing = PairingSession::new();

    let m1 = pairing.start_transient_pairing_with_pin(pin)?;
    let pair_setup_req = RtspRequest::pair_setup(m1, &client_device_id, HKP_TRANSIENT);
    let m2_resp = rtsp.send(pair_setup_req).await?;

    let m2_body = m2_resp.body.as_deref().unwrap_or(&[]);
    let m3 = pairing.continue_transient_pairing(m2_body)?;

    if let Some(m3_data) = m3 {
        let pair_setup_req = RtspRequest::pair_setup(m3_data, &client_device_id, HKP_TRANSIENT);
        let m4_resp = rtsp.send(pair_setup_req).await?;
        let m4_body = m4_resp.body.as_deref().unwrap_or(&[]);
        pairing.continue_transient_pairing(m4_body)?;
    }

    let session_keys = pairing.take_session_keys().ok_or_else(|| {
        RtspError::SetupFailed("Missing session keys after transient pairing".into())
    })?;

    install_control_cipher(&mut rtsp, session_keys);
    probe_encrypted_options(&mut rtsp).await;

    tracing::debug!("Using random shk for audio encryption (sent in SETUP phase 2)");

    session.set_paired()?;

    Ok(Connection::from_paired_parts(device, rtsp, session, config))
}

pub(crate) async fn connect_auto(
    device: Device,
    config: StreamConfig,
    fallback_pin: &str,
) -> Result<Connection> {
    connect_with_pin(device, config, fallback_pin).await
}

async fn open_connected_session(
    device: &Device,
    config: &StreamConfig,
    client_device_id: &str,
    context: Option<&str>,
) -> Result<(RtspConnection, RtspSession)> {
    let ip_addr = select_best_address(&device.addresses).ok_or(RtspError::ConnectionRefused)?;
    let addr = SocketAddr::new(*ip_addr, device.port);
    if let Some(context) = context {
        info!("Connecting to {} at {} ({})", device.name, addr, context);
    } else {
        info!("Connecting to {} at {}", device.name, addr);
    }

    let mut rtsp = RtspConnection::new(addr);
    rtsp.connect().await?;

    let mut session = RtspSession::new(device.clone(), config.clone());
    session.set_client_device_id(client_device_id.to_string());
    session.set_connected()?;
    session.set_request_host(ip_addr.to_string());

    add_default_rtsp_headers(&mut rtsp, client_device_id);

    let info_req = RtspRequest::get_info();
    let _info_resp = rtsp.send(info_req).await?;

    Ok((rtsp, session))
}

fn add_default_rtsp_headers(rtsp: &mut RtspConnection, client_device_id: &str) {
    let client_instance = client_device_id.replace(":", "");
    rtsp.add_session_header("User-Agent", AIRPLAY_USER_AGENT);
    rtsp.add_session_header("X-Apple-Client-Name", AIRPLAY_CLIENT_NAME);
    rtsp.add_session_header("X-Apple-Device-ID", client_device_id.to_string());
    rtsp.add_session_header("DACP-ID", client_instance.clone());
    rtsp.add_session_header("Client-Instance", client_instance);
    rtsp.add_session_header("Active-Remote", AIRPLAY_ACTIVE_REMOTE);
}

fn install_control_cipher(
    rtsp: &mut RtspConnection,
    session_keys: crate::crypto::keys::SessionKeys,
) {
    let cipher = ControlCipher::new(
        *session_keys.write_key.as_bytes(),
        *session_keys.read_key.as_bytes(),
    );
    rtsp.set_cipher(cipher);
}

async fn probe_encrypted_options(rtsp: &mut RtspConnection) {
    match rtsp.send(RtspRequest::options()).await {
        Ok(resp) => {
            if let Some(public) = resp.header("Public") {
                info!("OPTIONS supported methods: {}", public);
            } else {
                info!("OPTIONS returned {} (no Public header)", resp.status_code);
                for (key, value) in &resp.headers {
                    debug!("  OPTIONS header: {}: {}", key, value);
                }
            }
        }
        Err(error) => {
            warn!("Encrypted OPTIONS request failed: {}", error);
        }
    }
}
