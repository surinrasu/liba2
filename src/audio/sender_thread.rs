use crossbeam_channel::Receiver;
use std::net::{SocketAddr, UdpSocket};

#[cfg(target_os = "linux")]
pub(super) fn set_realtime_priority() {
    use std::mem;
    unsafe {
        let param: libc::sched_param = mem::zeroed();
        let mut param = param;
        param.sched_priority = 50; // RT priority 50 (1-99 scale)

        let result = libc::sched_setscheduler(
            0, // current thread
            libc::SCHED_FIFO,
            &param as *const _,
        );

        if result == 0 {
            tracing::info!("Set real-time priority (SCHED_FIFO, priority 50)");
        } else {
            tracing::warn!(
                "Failed to set RT priority (need CAP_SYS_NICE or root): errno={}",
                *libc::__errno_location()
            );
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn set_realtime_priority() {
    tracing::debug!("RT priority not supported on this platform");
}

#[cfg(target_os = "linux")]
fn disable_wifi_power_save() {
    use std::process::Command;
    for iface in &["wlan0", "wlp2s0", "wlp3s0"] {
        match Command::new("iw")
            .args([*iface, "set", "power_save", "off"])
            .output()
        {
            Ok(output) if output.status.success() => {
                tracing::info!("Disabled WiFi power save on {}", iface);
                return;
            }
            _ => {}
        }
    }
    tracing::debug!(
        "Could not disable WiFi power save (no wireless interface found or no permissions)"
    );
}

#[cfg(not(target_os = "linux"))]
fn disable_wifi_power_save() {}

pub(super) enum SenderMessage {
    Packet {
        wire_packets: Vec<Vec<u8>>,
        sync_data: Option<Vec<u8>>,
    },
    Pause,
    Resume,
    Stop,
}

#[cfg(target_os = "linux")]
fn precise_sleep_until(deadline_ns: u64) {
    let ts = libc::timespec {
        tv_sec: (deadline_ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (deadline_ns % 1_000_000_000) as libc::c_long,
    };

    unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            1, // TIMER_ABSTIME
            &ts as *const libc::timespec,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(target_os = "linux")]
fn monotonic_now_ns() -> u64 {
    use std::mem;
    unsafe {
        let mut ts: libc::timespec = mem::zeroed();
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts as *mut _);
        ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
    }
}

pub(super) struct SendTarget {
    pub(super) data_socket: UdpSocket,
    pub(super) data_dest: SocketAddr,
    pub(super) control_socket: Option<UdpSocket>,
    pub(super) control_dest: Option<SocketAddr>,
}

type BurstPacket = (Vec<Vec<u8>>, Option<Vec<u8>>);

pub(super) fn sender_thread_main(
    rx: Receiver<SenderMessage>,
    targets: Vec<SendTarget>,
    frame_duration: std::time::Duration,
    burst_size: usize,
) {
    set_realtime_priority();
    disable_wifi_power_save();

    let burst_size = burst_size.max(1);
    #[cfg(target_os = "linux")]
    let frame_duration_ns = frame_duration.as_nanos() as u64;
    #[cfg(target_os = "linux")]
    let burst_duration_ns = frame_duration_ns * burst_size as u64;
    #[cfg(target_os = "linux")]
    let mut next_deadline_ns = monotonic_now_ns();
    #[cfg(not(target_os = "linux"))]
    let sleeper = spin_sleep::SpinSleeper::default();
    #[cfg(not(target_os = "linux"))]
    let mut next_deadline = std::time::Instant::now();

    let mut last_send = std::time::Instant::now();
    let mut started = false;
    let mut packet_count: u64 = 0;
    let mut max_jitter_ms: f64 = 0.0;
    let mut jitter_sum_ms: f64 = 0.0;
    let mut jitter_exceed_count: u64 = 0;
    let mut burst_buffer: Vec<BurstPacket> = Vec::with_capacity(burst_size);

    tracing::info!(
        "Sender thread started: {} target(s), frame_duration={:.3}ms, burst_size={}, timing={}",
        targets.len(),
        frame_duration.as_secs_f64() * 1000.0,
        burst_size,
        if cfg!(target_os = "linux") {
            "clock_nanosleep(TIMER_ABSTIME)"
        } else {
            "spin_sleep"
        }
    );

    loop {
        let msg = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(msg) => msg,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                tracing::info!("Sender thread: channel disconnected, exiting");
                break;
            }
        };

        match msg {
            SenderMessage::Stop => {
                tracing::info!("Sender thread: received Stop, exiting");
                break;
            }
            SenderMessage::Pause => {
                tracing::debug!("Sender thread: paused");
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(SenderMessage::Resume) => {
                            tracing::debug!("Sender thread: resumed");
                            #[cfg(target_os = "linux")]
                            {
                                next_deadline_ns = monotonic_now_ns();
                            }
                            #[cfg(not(target_os = "linux"))]
                            {
                                next_deadline = std::time::Instant::now();
                            }
                            break;
                        }
                        Ok(SenderMessage::Stop) => {
                            tracing::info!("Sender thread: received Stop while paused, exiting");
                            return;
                        }
                        _ => continue,
                    }
                }
                continue;
            }
            SenderMessage::Resume => {
                #[cfg(target_os = "linux")]
                {
                    next_deadline_ns = monotonic_now_ns();
                }
                #[cfg(not(target_os = "linux"))]
                {
                    next_deadline = std::time::Instant::now();
                }
                continue;
            }
            SenderMessage::Packet {
                wire_packets,
                sync_data,
            } => {
                burst_buffer.push((wire_packets, sync_data));

                if burst_buffer.len() < burst_size && started {
                    continue;
                }

                if !started {
                    #[cfg(target_os = "linux")]
                    {
                        next_deadline_ns = monotonic_now_ns();
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        next_deadline = std::time::Instant::now();
                    }
                    last_send = std::time::Instant::now();
                    started = true;
                } else {
                    #[cfg(target_os = "linux")]
                    {
                        precise_sleep_until(next_deadline_ns);
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        let now = std::time::Instant::now();
                        if next_deadline > now {
                            sleeper.sleep(next_deadline - now);
                        }
                    }
                }

                let send_time = std::time::Instant::now();
                let actual_interval = send_time.duration_since(last_send);
                let interval_ms = actual_interval.as_secs_f64() * 1000.0;
                let target_ms = frame_duration.as_secs_f64() * 1000.0 * burst_buffer.len() as f64;
                let jitter_ms = interval_ms - target_ms;

                packet_count += burst_buffer.len() as u64;
                if packet_count > burst_size as u64 {
                    let abs_jitter = jitter_ms.abs();
                    if abs_jitter > max_jitter_ms {
                        max_jitter_ms = abs_jitter;
                    }
                    jitter_sum_ms += abs_jitter;
                    if abs_jitter > 1.0 * burst_size as f64 {
                        jitter_exceed_count += 1;
                    }

                    if abs_jitter > 2.0 * burst_size as f64 {
                        tracing::warn!(
                            "JITTER burst#{}: interval={:.3}ms target={:.3}ms jitter={:+.3}ms",
                            packet_count / burst_size as u64,
                            interval_ms,
                            target_ms,
                            jitter_ms
                        );
                    }

                    if packet_count % 500 < burst_size as u64 {
                        let bursts = packet_count / burst_size as u64;
                        let avg_jitter = if bursts > 1 {
                            jitter_sum_ms / (bursts - 1) as f64
                        } else {
                            0.0
                        };
                        tracing::info!(
                            "TIMING STATS after {} pkts ({} bursts): avg_jitter={:.3}ms max_jitter={:.3}ms exceeds={}",
                            packet_count,
                            bursts,
                            avg_jitter,
                            max_jitter_ms,
                            jitter_exceed_count
                        );
                    }
                }

                last_send = send_time;

                for (wire_packets, sync_data) in burst_buffer.drain(..) {
                    if let Some(ref sync) = sync_data {
                        for target in &targets {
                            let sock = target
                                .control_socket
                                .as_ref()
                                .unwrap_or(&target.data_socket);
                            let dest = target.control_dest.unwrap_or(target.data_dest);
                            if let Err(error) = sock.send_to(sync, dest) {
                                tracing::error!(
                                    "Failed to send sync packet to {}: {}",
                                    dest,
                                    error
                                );
                            }
                        }
                    }

                    for (index, target) in targets.iter().enumerate() {
                        if let Some(wire_data) = wire_packets.get(index)
                            && let Err(error) =
                                target.data_socket.send_to(wire_data, target.data_dest)
                        {
                            tracing::error!(
                                "Failed to send audio packet to {}: {}",
                                target.data_dest,
                                error
                            );
                        }
                    }
                }

                #[cfg(target_os = "linux")]
                {
                    next_deadline_ns += burst_duration_ns;
                    let now_ns = monotonic_now_ns();
                    if now_ns > next_deadline_ns + burst_duration_ns {
                        tracing::warn!(
                            "Sender thread fell behind by {:.2}ms, resetting deadline",
                            (now_ns - next_deadline_ns) as f64 / 1_000_000.0
                        );
                        next_deadline_ns = now_ns;
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    next_deadline += frame_duration * burst_size as u32;
                    let now = std::time::Instant::now();
                    if now > next_deadline + frame_duration * burst_size as u32 {
                        tracing::warn!(
                            "Sender thread fell behind by {:.2}ms, resetting deadline",
                            (now - next_deadline).as_secs_f64() * 1000.0
                        );
                        next_deadline = now;
                    }
                }
            }
        }
    }

    tracing::info!("Sender thread exiting");
}
