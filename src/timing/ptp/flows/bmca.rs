use super::super::messages::{
    send_mac_style_signaling, send_ptp_announce, send_ptp_sync, send_stop_signaling,
};
use super::super::types::{
    PTP_EVENT_PORT, PTP_GENERAL_PORT, PtpHeader, PtpMessageType, PtpTimestamp,
};
use crate::core::error::Result;
use crate::timing::ClockOffset;

pub async fn run_bmca_yield_flow(
    master_ip: std::net::IpAddr,
    priority1: u8,
    offset_tx: tokio::sync::watch::Sender<ClockOffset>,
    clock_id_tx: tokio::sync::oneshot::Sender<[u8; 8]>,
) -> Result<()> {
    use tokio::net::UdpSocket;

    let event_socket = match UdpSocket::bind(("0.0.0.0", PTP_EVENT_PORT)).await {
        Ok(s) => {
            tracing::info!("BMCA: bound to event port {}", PTP_EVENT_PORT);
            s
        }
        Err(_) => {
            let s = UdpSocket::bind("0.0.0.0:0").await?;
            tracing::warn!(
                "BMCA: using ephemeral event port {}",
                s.local_addr()?.port()
            );
            s
        }
    };

    let general_socket = match UdpSocket::bind(("0.0.0.0", PTP_GENERAL_PORT)).await {
        Ok(s) => {
            tracing::info!("BMCA: bound to general port {}", PTP_GENERAL_PORT);
            s
        }
        Err(_) => {
            let s = UdpSocket::bind("0.0.0.0:0").await?;
            tracing::warn!(
                "BMCA: using ephemeral general port {}",
                s.local_addr()?.port()
            );
            s
        }
    };

    let event_dest = std::net::SocketAddr::new(master_ip, PTP_EVENT_PORT);
    let general_dest = std::net::SocketAddr::new(master_ip, PTP_GENERAL_PORT);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let clock_identity = (now as u64).to_be_bytes();

    let mut sync_seq: u16 = 0;
    let mut announce_seq: u16 = 0;
    let mut signaling_seq: u16 = 0;

    tracing::info!(
        "BMCA: Starting Mac-style negotiation with {} (our priority1={})",
        master_ip,
        priority1
    );

    for i in 0..3 {
        send_ptp_sync(
            &event_socket,
            &general_socket,
            event_dest,
            &clock_identity,
            &mut sync_seq,
        )
        .await?;
        if i < 2 {
            send_ptp_announce(
                &general_socket,
                general_dest,
                &clock_identity,
                &mut announce_seq,
                248,
                priority1,
            )
            .await?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(125)).await;
    }
    send_mac_style_signaling(
        &general_socket,
        general_dest,
        &clock_identity,
        &mut signaling_seq,
    )
    .await?;

    tracing::info!("BMCA: Initial messages sent, waiting for remote Announce...");

    let mut remote_clock_id = [0u8; 8];
    let mut remote_priority1: u8 = 255;
    let mut general_buf = [0u8; 256];
    let mut event_buf = [0u8; 256];
    let bmca_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(bmca_deadline) => {
                tracing::warn!("BMCA: Timeout waiting for remote Announce, proceeding as slave anyway");
                break;
            }
            result = general_socket.recv_from(&mut general_buf) => {
                if let Ok((len, src)) = result {
                    if src.ip() != master_ip {
                        tracing::debug!("BMCA: Ignoring PTP general from {} (not master {})", src.ip(), master_ip);
                        continue;
                    }
                    if let Ok(header) = PtpHeader::parse(&general_buf[..len])
                        && header.message_type == PtpMessageType::Announce
                        && len >= 61
                    {
                        remote_priority1 = general_buf[47];
                        remote_clock_id.copy_from_slice(&general_buf[53..61]);
                        tracing::info!("BMCA: Received Announce from {}: priority1={}, clock_id={:02x?}",
                            src.ip(), remote_priority1, remote_clock_id);
                        break;
                    }
                }
            }
            result = event_socket.recv_from(&mut event_buf) => {
                if let Ok((len, src)) = result {
                    if src.ip() != master_ip {
                        tracing::debug!("BMCA: Ignoring PTP event from {} (not master {})", src.ip(), master_ip);
                        continue;
                    }
                    if let Ok(header) = PtpHeader::parse(&event_buf[..len]) {
                        tracing::debug!("BMCA: Received {:?} on event port during negotiation from {}", header.message_type, src.ip());
                    }
                }
            }
        }
    }

    let we_are_master = priority1 < remote_priority1;
    tracing::info!(
        "BMCA: Decision -- our_p1={}, remote_p1={}, we_are_master={}",
        priority1,
        remote_priority1,
        we_are_master
    );

    if !we_are_master {
        send_stop_signaling(
            &general_socket,
            general_dest,
            &clock_identity,
            &mut signaling_seq,
        )
        .await?;
        tracing::info!("BMCA: Yielded master to remote, transitioning to slave mode");
    }

    let _ = clock_id_tx.send(remote_clock_id);

    let mut t1: Option<PtpTimestamp> = None;
    let mut t2: Option<PtpTimestamp> = None;
    let mut t3: Option<PtpTimestamp> = None;
    let mut delay_req_seq: u16 = 0;

    tracing::info!(
        "BMCA: Entering slave loop, syncing to {} (filtering other peers)",
        master_ip
    );

    loop {
        tokio::select! {
            result = event_socket.recv_from(&mut event_buf) => {
                if let Ok((len, src)) = result {
                    if src.ip() != master_ip {
                        tracing::trace!("BMCA slave: Ignoring event from {} (not master {})", src.ip(), master_ip);
                        continue;
                    }
                    if let Ok(header) = PtpHeader::parse(&event_buf[..len]) {
                        match header.message_type {
                            PtpMessageType::Sync => {
                                t2 = Some(PtpTimestamp::now());
                                tracing::trace!("BMCA slave: Received Sync from {} (seq={})", src.ip(), header.sequence_id);
                            }
                            PtpMessageType::DelayResp if len >= 44 => {
                                if let Ok(t4) = PtpTimestamp::parse(&event_buf[34..44]) {
                                    if let (Some(t1v), Some(t2v), Some(t3v)) = (t1, t2, t3) {
                                        let t1_ns = t1v.to_nanos() as i128;
                                        let t2_ns = t2v.to_nanos() as i128;
                                        let t3_ns = t3v.to_nanos() as i128;
                                        let t4_ns = t4.to_nanos() as i128;

                                        let offset_val = ((t2_ns - t1_ns) + (t3_ns - t4_ns)) / 2;
                                        let delay = ((t2_ns - t1_ns) - (t3_ns - t4_ns)) / 2;

                                        let clock_offset = ClockOffset {
                                            offset_ns: offset_val as i64,
                                            error_ns: (delay.unsigned_abs() / 2) as u64,
                                            rtt_ns: delay.unsigned_abs() as u64,
                                        };

                                        tracing::debug!(
                                            "BMCA slave: synchronized offset={}ns, delay={}ns",
                                            clock_offset.offset_ns, clock_offset.rtt_ns
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
            }
            result = general_socket.recv_from(&mut general_buf) => {
                if let Ok((len, src)) = result {
                    if src.ip() != master_ip {
                        tracing::trace!("BMCA slave: Ignoring general from {} (not master {})", src.ip(), master_ip);
                        continue;
                    }
                    if let Ok(header) = PtpHeader::parse(&general_buf[..len])
                        && header.message_type == PtpMessageType::FollowUp
                        && len >= 44
                        && let Ok(ts) = PtpTimestamp::parse(&general_buf[34..44])
                    {
                        t1 = Some(ts);
                        tracing::trace!("BMCA slave: Received Follow_Up from {} (seq={}, t1={}.{:09}s)",
                            src.ip(), header.sequence_id, ts.seconds, ts.nanoseconds);

                        delay_req_seq = delay_req_seq.wrapping_add(1);
                        let mut delay_header = PtpHeader::new(PtpMessageType::DelayReq, delay_req_seq);
                        let mut delay_src_port = [0u8; 10];
                        delay_src_port[..8].copy_from_slice(&clock_identity);
                        delay_src_port[8..10].copy_from_slice(&1u16.to_be_bytes());
                        delay_header.source_port_identity = delay_src_port;

                        t3 = Some(PtpTimestamp::now());

                        let mut delay_packet = [0u8; 44];
                        delay_packet[..34].copy_from_slice(&delay_header.serialize());
                        delay_packet[34..44].copy_from_slice(&t3.unwrap().serialize());

                        if let Err(e) = event_socket.send_to(&delay_packet, event_dest).await {
                            tracing::warn!("BMCA slave: Failed to send Delay_Req: {}", e);
                        } else {
                            tracing::trace!("BMCA slave: Sent Delay_Req (seq={})", delay_req_seq);
                        }
                    }
                }
            }
        }
    }
}
