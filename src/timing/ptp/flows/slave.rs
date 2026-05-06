use super::super::messages::{send_ptp_announce, send_ptp_signaling};
use super::super::types::{
    PTP_EVENT_PORT, PTP_GENERAL_PORT, PtpHeader, PtpMessageType, PtpTimestamp,
};
use crate::core::error::Result;
use crate::timing::ClockOffset;

pub async fn run_ptp_slave(
    master_ip: std::net::IpAddr,
    offset_tx: tokio::sync::watch::Sender<ClockOffset>,
) -> Result<()> {
    use tokio::net::UdpSocket;

    let event_socket = match UdpSocket::bind(("0.0.0.0", PTP_EVENT_PORT)).await {
        Ok(s) => {
            tracing::info!("PTP slave bound to event port {}", PTP_EVENT_PORT);
            s
        }
        Err(_) => {
            let s = UdpSocket::bind("0.0.0.0:0").await?;
            tracing::warn!(
                "PTP slave using ephemeral event port {}",
                s.local_addr()?.port()
            );
            s
        }
    };

    let general_socket = match UdpSocket::bind(("0.0.0.0", PTP_GENERAL_PORT)).await {
        Ok(s) => {
            tracing::info!("PTP slave bound to general port {}", PTP_GENERAL_PORT);
            s
        }
        Err(_) => {
            let s = UdpSocket::bind("0.0.0.0:0").await?;
            tracing::warn!(
                "PTP slave using ephemeral general port {}",
                s.local_addr()?.port()
            );
            s
        }
    };

    let master_event = std::net::SocketAddr::new(master_ip, PTP_EVENT_PORT);
    let master_general = std::net::SocketAddr::new(master_ip, PTP_GENERAL_PORT);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let clock_identity = (now as u64).to_be_bytes();

    let mut t1: Option<PtpTimestamp> = None;
    let mut t2: Option<PtpTimestamp> = None;
    let mut t3: Option<PtpTimestamp> = None;
    let mut sequence_id: u16 = 0;
    let mut announce_seq: u16 = 0;
    let mut signaling_seq: u16 = 0;

    let mut event_buf = [0u8; 256];
    let mut general_buf = [0u8; 256];

    tracing::info!(
        "PTP slave started, listening for Sync from master {}",
        master_ip
    );

    tracing::info!("gPTP slave: Sending initial Announce+Signaling handshake");

    if let Err(e) = send_ptp_announce(
        &general_socket,
        master_general,
        &clock_identity,
        &mut announce_seq,
        255,
        255,
    )
    .await
    {
        tracing::warn!("Failed to send initial Announce: {}", e);
    }

    if let Err(e) = send_ptp_signaling(
        &general_socket,
        master_general,
        &clock_identity,
        &mut signaling_seq,
        -3,
        -2,
    )
    .await
    {
        tracing::warn!("Failed to send initial Signaling: {}", e);
    }

    tracing::info!("gPTP slave: Handshake complete, waiting for master's Sync messages");

    let mut announce_interval = tokio::time::interval(std::time::Duration::from_millis(250));
    announce_interval.tick().await;

    loop {
        tokio::select! {
            _ = announce_interval.tick() => {
                if let Err(e) = send_ptp_announce(
                    &general_socket,
                    master_general,
                    &clock_identity,
                    &mut announce_seq,
                    255,
                    255,
                ).await {
                    tracing::warn!("Failed to send periodic Announce: {}", e);
                } else {
                    tracing::trace!("gPTP slave: Sent periodic Announce (seq={})", announce_seq);
                }
            }
            result = event_socket.recv_from(&mut event_buf) => {
                match result {
                    Ok((len, _src)) => {
                        if let Ok(header) = PtpHeader::parse(&event_buf[..len]) {
                            match header.message_type {
                                PtpMessageType::Sync => {
                                    t2 = Some(PtpTimestamp::now());
                                    tracing::trace!(
                                        "PTP: Received Sync (seq={})",
                                        header.sequence_id
                                    );
                                }
                                PtpMessageType::DelayResp if len >= 44 => {
                                    if let Ok(t4) = PtpTimestamp::parse(&event_buf[34..44]) {
                                        if let (Some(t1v), Some(t2v), Some(t3v)) = (t1, t2, t3) {
                                            let t1_ns = t1v.to_nanos() as i128;
                                            let t2_ns = t2v.to_nanos() as i128;
                                            let t3_ns = t3v.to_nanos() as i128;
                                            let t4_ns = t4.to_nanos() as i128;

                                            let offset_val =
                                                ((t2_ns - t1_ns) + (t3_ns - t4_ns)) / 2;
                                            let delay =
                                                ((t2_ns - t1_ns) - (t3_ns - t4_ns)) / 2;

                                            let clock_offset = ClockOffset {
                                                offset_ns: offset_val as i64,
                                                error_ns: (delay.unsigned_abs() / 2) as u64,
                                                rtt_ns: delay.unsigned_abs() as u64,
                                            };

                                            tracing::debug!(
                                                "PTP synchronized: offset={}ns, delay={}ns",
                                                clock_offset.offset_ns,
                                                clock_offset.rtt_ns
                                            );
                                            let _ = offset_tx.send(clock_offset);
                                        }
                                        t1 = None;
                                        t2 = None;
                                        t3 = None;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("PTP event socket error: {}", e);
                    }
                }
            }
            result = general_socket.recv_from(&mut general_buf) => {
                match result {
                    Ok((len, _src)) => {
                        if let Ok(header) = PtpHeader::parse(&general_buf[..len])
                            && header.message_type == PtpMessageType::FollowUp
                            && len >= 44
                            && let Ok(ts) = PtpTimestamp::parse(&general_buf[34..44])
                        {
                            t1 = Some(ts);
                            tracing::trace!(
                                "PTP: Received Follow_Up (seq={}, t1={:?})",
                                header.sequence_id,
                                ts
                            );

                            sequence_id = sequence_id.wrapping_add(1);
                            let delay_header =
                                PtpHeader::new(PtpMessageType::DelayReq, sequence_id);

                            t3 = Some(PtpTimestamp::now());

                            let mut delay_packet = [0u8; 44];
                            delay_packet[..34].copy_from_slice(&delay_header.serialize());
                            delay_packet[34..44].copy_from_slice(&t3.unwrap().serialize());

                            if let Err(e) = event_socket.send_to(&delay_packet, master_event).await
                            {
                                tracing::warn!("Failed to send Delay_Req: {}", e);
                            } else {
                                tracing::trace!("PTP: Sent Delay_Req (seq={})", sequence_id);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("PTP general socket error: {}", e);
                    }
                }
            }
        }
    }
}
