use super::tlv::{
    AppleTlv, FollowUpInformationTlv, MessageIntervalRequestTlv, apple_subtype, gptp_subtype,
    org_id,
};
use super::types::{PTP_GENERAL_PORT, PtpHeader, PtpMessageType, PtpTimestamp};
use crate::core::error::Result;

pub async fn send_ptp_announce(
    general_socket: &tokio::net::UdpSocket,
    dest: std::net::SocketAddr,
    clock_identity: &[u8; 8],
    announce_seq: &mut u16,
    clock_class: u8,
    priority1: u8,
) -> Result<()> {
    *announce_seq = announce_seq.wrapping_add(1);
    let seq = *announce_seq;

    let mut source_port_identity = [0u8; 10];
    source_port_identity[..8].copy_from_slice(clock_identity);
    source_port_identity[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut header = PtpHeader::new(PtpMessageType::Announce, seq);
    header.source_port_identity = source_port_identity;
    header.message_length = 64;
    header.log_message_interval = 1;

    let mut packet = [0u8; 64];
    packet[..34].copy_from_slice(&header.serialize());

    packet[44..46].copy_from_slice(&37i16.to_be_bytes());
    packet[47] = priority1;
    packet[48] = clock_class;
    packet[49] = 0xFE;
    packet[50..52].copy_from_slice(&0xFFFFu16.to_be_bytes());
    packet[52] = 128;
    packet[53..61].copy_from_slice(clock_identity);
    packet[63] = 0xA0;

    let general_dest = std::net::SocketAddr::new(dest.ip(), PTP_GENERAL_PORT);
    general_socket.send_to(&packet, general_dest).await?;
    tracing::info!(
        "PTP: Sent Announce to {}:{} (seq={}, clockClass={})",
        general_dest.ip(),
        general_dest.port(),
        seq,
        clock_class
    );

    Ok(())
}

pub async fn send_ptp_signaling(
    general_socket: &tokio::net::UdpSocket,
    dest: std::net::SocketAddr,
    clock_identity: &[u8; 8],
    sequence_id: &mut u16,
    sync_interval_log: i8,
    announce_interval_log: i8,
) -> Result<()> {
    *sequence_id = sequence_id.wrapping_add(1);
    let seq = *sequence_id;

    let mut source_port_identity = [0u8; 10];
    source_port_identity[..8].copy_from_slice(clock_identity);
    source_port_identity[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut header = PtpHeader::new(PtpMessageType::Signaling, seq);
    header.source_port_identity = source_port_identity;

    let tlv = MessageIntervalRequestTlv::new(sync_interval_log, announce_interval_log);
    let tlv_bytes = tlv.serialize();

    let apple_tlv_01_payload = vec![
        0x00,
        0x04,
        sync_interval_log as u8,
        announce_interval_log as u8,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ];
    let apple_tlv_01 = AppleTlv::new(apple_subtype::TYPE_01, apple_tlv_01_payload);
    let apple_tlv_01_bytes = apple_tlv_01.serialize();

    let apple_tlv_05_payload = vec![
        0x00,
        0x10,
        sync_interval_log as u8,
        announce_interval_log as u8,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ];
    let apple_tlv_05 = AppleTlv::new(apple_subtype::TYPE_05, apple_tlv_05_payload);
    let apple_tlv_05_bytes = apple_tlv_05.serialize();

    header.message_length = 34
        + 10
        + tlv_bytes.len() as u16
        + apple_tlv_01_bytes.len() as u16
        + apple_tlv_05_bytes.len() as u16;

    let mut packet = Vec::new();
    packet.extend_from_slice(&header.serialize());
    packet.extend_from_slice(&[0xFF; 10]);
    packet.extend_from_slice(&tlv_bytes);
    packet.extend_from_slice(&apple_tlv_01_bytes);
    packet.extend_from_slice(&apple_tlv_05_bytes);

    let general_dest = std::net::SocketAddr::new(dest.ip(), PTP_GENERAL_PORT);
    general_socket.send_to(&packet, general_dest).await?;
    tracing::info!(
        "gPTP: Sent Signaling to {}:{} (seq={}, sync={}ms, announce={}ms)",
        general_dest.ip(),
        general_dest.port(),
        seq,
        1000 / (1 << (-sync_interval_log)),
        1000 / (1 << (-announce_interval_log))
    );

    Ok(())
}

pub async fn send_ptp_sync(
    event_socket: &tokio::net::UdpSocket,
    general_socket: &tokio::net::UdpSocket,
    dest: std::net::SocketAddr,
    clock_identity: &[u8; 8],
    sequence_id: &mut u16,
) -> Result<()> {
    *sequence_id = sequence_id.wrapping_add(1);
    let seq = *sequence_id;

    let mut source_port_identity = [0u8; 10];
    source_port_identity[..8].copy_from_slice(clock_identity);
    source_port_identity[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut sync_header = PtpHeader::new(PtpMessageType::Sync, seq);
    sync_header.flags = 0x0200;
    sync_header.source_port_identity = source_port_identity;

    let mut sync_packet = [0u8; 44];
    sync_packet[..34].copy_from_slice(&sync_header.serialize());

    event_socket.send_to(&sync_packet, dest).await?;
    let sync_time = PtpTimestamp::now();
    tracing::debug!(
        "gPTP: Sent Sync to {}:{} (seq={})",
        dest.ip(),
        dest.port(),
        seq
    );

    let mut followup_header = PtpHeader::new(PtpMessageType::FollowUp, seq);
    followup_header.source_port_identity = source_port_identity;

    let followup_tlv = FollowUpInformationTlv::new();
    let tlv_bytes = followup_tlv.serialize();

    followup_header.message_length = 44 + tlv_bytes.len() as u16;

    let mut followup_packet = Vec::new();
    followup_packet.extend_from_slice(&followup_header.serialize());
    followup_packet.extend_from_slice(&sync_time.serialize());
    followup_packet.extend_from_slice(&tlv_bytes);

    let general_dest = std::net::SocketAddr::new(dest.ip(), PTP_GENERAL_PORT);
    general_socket
        .send_to(&followup_packet, general_dest)
        .await?;
    tracing::debug!(
        "gPTP: Sent Follow_Up to {}:{} (seq={}, t1={}.{:09}s, with TLV)",
        general_dest.ip(),
        general_dest.port(),
        seq,
        sync_time.seconds,
        sync_time.nanoseconds
    );

    Ok(())
}

pub async fn send_mac_style_signaling(
    general_socket: &tokio::net::UdpSocket,
    dest: std::net::SocketAddr,
    clock_identity: &[u8; 8],
    sequence_id: &mut u16,
) -> Result<()> {
    *sequence_id = sequence_id.wrapping_add(1);
    let seq = *sequence_id;

    let mut source_port_identity = [0u8; 10];
    source_port_identity[..8].copy_from_slice(clock_identity);
    source_port_identity[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut header = PtpHeader::new(PtpMessageType::Signaling, seq);
    header.source_port_identity = source_port_identity;

    let tlv = MessageIntervalRequestTlv::new(-3, -2);
    let tlv_bytes = tlv.serialize();

    let apple_tlv_01_payload = vec![
        0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x27, 0x10, 0x00, 0x00, 0x27, 0x10, 0x00, 0x00, 0x00,
        0x00,
    ];
    let apple_tlv_01 = AppleTlv::new(apple_subtype::TYPE_01, apple_tlv_01_payload);
    let apple_tlv_01_bytes = apple_tlv_01.serialize();

    let apple_tlv_05_payload = vec![
        0x00, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x27, 0x10, 0x00, 0x00, 0x27, 0x10, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let apple_tlv_05 = AppleTlv::new(apple_subtype::TYPE_05, apple_tlv_05_payload);
    let apple_tlv_05_bytes = apple_tlv_05.serialize();

    header.message_length = 34
        + 10
        + tlv_bytes.len() as u16
        + apple_tlv_01_bytes.len() as u16
        + apple_tlv_05_bytes.len() as u16;

    let mut packet = Vec::new();
    packet.extend_from_slice(&header.serialize());
    packet.extend_from_slice(&[0xFF; 10]);
    packet.extend_from_slice(&tlv_bytes);
    packet.extend_from_slice(&apple_tlv_01_bytes);
    packet.extend_from_slice(&apple_tlv_05_bytes);

    let general_dest = std::net::SocketAddr::new(dest.ip(), PTP_GENERAL_PORT);
    general_socket.send_to(&packet, general_dest).await?;
    tracing::info!(
        "gPTP: Sent Mac-style Signaling to {} (seq={}, Apple TLVs with 0x2710)",
        general_dest,
        seq
    );

    Ok(())
}

pub async fn send_stop_signaling(
    general_socket: &tokio::net::UdpSocket,
    dest: std::net::SocketAddr,
    clock_identity: &[u8; 8],
    sequence_id: &mut u16,
) -> Result<()> {
    *sequence_id = sequence_id.wrapping_add(1);
    let seq = *sequence_id;

    let mut source_port_identity = [0u8; 10];
    source_port_identity[..8].copy_from_slice(clock_identity);
    source_port_identity[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut header = PtpHeader::new(PtpMessageType::Signaling, seq);
    header.source_port_identity = source_port_identity;

    let stop_tlv = MessageIntervalRequestTlv {
        organization_id: org_id::GPTP,
        organization_subtype: gptp_subtype::MESSAGE_INTERVAL_REQUEST,
        link_delay_interval: 0x7E,
        time_sync_interval: 0x7E,
        announce_interval: 0x7E,
        flags: 0x00,
    };
    let tlv_bytes = stop_tlv.serialize();

    header.message_length = 34 + 10 + tlv_bytes.len() as u16;

    let mut packet = Vec::new();
    packet.extend_from_slice(&header.serialize());
    packet.extend_from_slice(&[0xFF; 10]);
    packet.extend_from_slice(&tlv_bytes);

    let general_dest = std::net::SocketAddr::new(dest.ip(), PTP_GENERAL_PORT);
    general_socket.send_to(&packet, general_dest).await?;
    tracing::info!(
        "gPTP: Sent STOP Signaling to {} (seq={}, all intervals=0x7E)",
        general_dest,
        seq
    );

    Ok(())
}
