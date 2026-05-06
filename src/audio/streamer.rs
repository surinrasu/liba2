use crate::audio::encoder::{AudioEncoder, create_encoder};
use crate::audio::sender_thread::{
    SendTarget, SenderMessage, sender_thread_main, set_realtime_priority,
};
use crate::audio::{AudioBuffer, LiveAudioDecoder, RtpSender};
use crate::core::{StreamConfig, error::Result};
use crate::protocol::next_airplay_packet_timestamp;
use crate::timing::{Clock, ClockOffset, unix_to_ntp};
use crossbeam_channel::{Sender, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep};

const DEFAULT_BUFFER_CAPACITY_MS: u32 = 2000;
const BUFFER_LOW_WATERMARK_PERCENT: f32 = 40.0;
const BUFFER_RESUME_FILL_PERCENT: f32 = 10.0;
const BUFFER_RETRY_SLEEP: Duration = Duration::from_millis(10);
const DECODE_BATCH_LIMIT: usize = 3;
const DECODE_WARNING_THRESHOLD_MS: u128 = 10;
const SENDER_CHANNEL_CAPACITY: usize = 8;
const DEFAULT_BURST_SIZE: usize = 1;
const NANOS_PER_MILLI: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamerState {
    Idle,
    Buffering,
    Streaming,
    Paused,
    Stopped,
    Error,
}

struct StreamerInner {
    state: StreamerState,
    config: StreamConfig,
    buffer: AudioBuffer,
    rtp_senders: Vec<RtpSender>,
    current_timestamp: u64,
    last_sync_rtp: u32,
    clock: Clock,
    clock_offset: Option<ClockOffset>,
    timing_rx: Option<watch::Receiver<ClockOffset>>,
    live_decoder: Option<LiveAudioDecoder>,
    encoder: Option<Box<dyn AudioEncoder>>,
    first_packet_sent: bool,
    render_delay_ns: u64,
    use_ptp_sync: bool,
    ptp_master_clock_id: [u8; 8],
}

pub struct AudioStreamer {
    inner: Arc<Mutex<StreamerInner>>,
    task: Option<JoinHandle<Result<()>>>,
    state_cache: Arc<AtomicU8>,
    timestamp_cache: Arc<AtomicU64>,
    packets_sent: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
    sender_thread: Option<std::thread::JoinHandle<()>>,
    sender_tx: Option<Sender<SenderMessage>>,
}

impl Clone for AudioStreamer {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            task: None, // Can't clone JoinHandle, new clone doesn't own the task
            state_cache: Arc::clone(&self.state_cache),
            timestamp_cache: Arc::clone(&self.timestamp_cache),
            packets_sent: Arc::clone(&self.packets_sent),
            underruns: Arc::clone(&self.underruns),
            sender_thread: None, // Can't clone JoinHandle
            sender_tx: self.sender_tx.clone(),
        }
    }
}

impl AudioStreamer {
    pub fn new(config: StreamConfig) -> Self {
        let audio_format = config.audio_format;
        Self {
            inner: Arc::new(Mutex::new(StreamerInner {
                state: StreamerState::Idle,
                config,
                buffer: AudioBuffer::new(audio_format, DEFAULT_BUFFER_CAPACITY_MS),
                rtp_senders: Vec::new(),
                current_timestamp: 0,
                last_sync_rtp: 0,
                clock: Clock::new(),
                clock_offset: None,
                timing_rx: None,
                live_decoder: None,
                encoder: None,
                first_packet_sent: false,
                render_delay_ns: 0,
                use_ptp_sync: false,
                ptp_master_clock_id: [0u8; 8],
            })),
            task: None,
            state_cache: Arc::new(AtomicU8::new(StreamerState::Idle as u8)),
            timestamp_cache: Arc::new(AtomicU64::new(0)),
            packets_sent: Arc::new(AtomicU64::new(0)),
            underruns: Arc::new(AtomicU64::new(0)),
            sender_thread: None,
            sender_tx: None,
        }
    }

    pub async fn set_rtp_sender(&mut self, sender: RtpSender) {
        self.inner.lock().await.rtp_senders = vec![sender];
    }

    pub async fn set_rtp_senders(&mut self, senders: Vec<RtpSender>) {
        self.inner.lock().await.rtp_senders = senders;
    }

    pub async fn set_ptp_sync_mode(&mut self, clock_id: [u8; 8]) {
        let mut inner = self.inner.lock().await;
        inner.use_ptp_sync = true;
        inner.ptp_master_clock_id = clock_id;
        tracing::info!("PTP sync mode enabled, master clock ID: {:02x?}", clock_id);
    }

    pub async fn set_render_delay_ms(&mut self, delay_ms: u32) {
        let delay_ns = delay_ms as u64 * NANOS_PER_MILLI;
        self.inner.lock().await.render_delay_ns = delay_ns;
        tracing::info!("Render delay set to {}ms ({}ns)", delay_ms, delay_ns);
    }

    pub async fn handle_retransmit(
        &self,
        request: &crate::audio::rtp::RetransmitRequest,
    ) -> crate::core::error::Result<u16> {
        self.handle_retransmit_for_target(0, request).await
    }

    pub async fn handle_retransmit_for_target(
        &self,
        index: usize,
        request: &crate::audio::rtp::RetransmitRequest,
    ) -> crate::core::error::Result<u16> {
        let inner = self.inner.lock().await;
        if let Some(sender) = inner.rtp_senders.get(index) {
            sender.handle_retransmit(request)
        } else {
            Ok(0)
        }
    }

    pub fn packets_sent(&self) -> u64 {
        self.packets_sent.load(Ordering::Relaxed)
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub async fn start_live(&mut self, live_decoder: LiveAudioDecoder) -> Result<()> {
        let frame_duration_ns;
        {
            let mut inner = self.inner.lock().await;
            inner.live_decoder = Some(live_decoder);
            inner.encoder = Some(create_encoder(inner.config.audio_format)?);
            inner.state = StreamerState::Buffering;
            frame_duration_ns = inner.config.audio_format.frames_per_packet as u64
                * 1_000_000_000u64
                / inner.config.audio_format.sample_rate.as_hz() as u64;
        }
        self.state_cache
            .store(StreamerState::Buffering as u8, Ordering::Relaxed);

        tracing::info!("Live streaming: starting async buffer fill");

        if self.task.is_none() {
            let (tx, rx) = bounded::<SenderMessage>(SENDER_CHANNEL_CAPACITY);
            let frame_duration = std::time::Duration::from_nanos(frame_duration_ns);

            {
                let inner = self.inner.lock().await;
                let mut targets = Vec::new();
                for sender in &inner.rtp_senders {
                    if let Ok(Some((data_sock, data_dest))) = sender.clone_data_socket() {
                        let ctrl = sender.clone_control_socket().ok().flatten();
                        let (ctrl_sock, ctrl_dest) = match ctrl {
                            Some((s, d)) => (Some(s), Some(d)),
                            None => (None, None),
                        };
                        targets.push(SendTarget {
                            data_socket: data_sock,
                            data_dest,
                            control_socket: ctrl_sock,
                            control_dest: ctrl_dest,
                        });
                    }
                }

                if !targets.is_empty() {
                    let burst_size = DEFAULT_BURST_SIZE;

                    let thread = std::thread::Builder::new()
                        .name("rt-sender".into())
                        .spawn(move || {
                            sender_thread_main(rx, targets, frame_duration, burst_size);
                        })
                        .expect("Failed to spawn sender thread");
                    self.sender_thread = Some(thread);
                    self.sender_tx = Some(tx);
                } else {
                    tracing::warn!("No RTP senders have sockets, falling back to async timing");
                }
            }

            let inner = self.inner.clone();
            let state_cache = self.state_cache.clone();
            let timestamp_cache = self.timestamp_cache.clone();
            let packets_sent = self.packets_sent.clone();
            let underruns = self.underruns.clone();
            let sender_tx = self.sender_tx.clone();
            self.task = Some(tokio::spawn(async move {
                match run_streamer(
                    inner.clone(),
                    state_cache.clone(),
                    timestamp_cache,
                    packets_sent,
                    underruns,
                    sender_tx,
                )
                .await
                {
                    Ok(()) => tracing::debug!("Live streaming task completed normally"),
                    Err(e) => {
                        tracing::error!("Live streaming task error: {}", e);
                        state_cache.store(StreamerState::Error as u8, Ordering::Relaxed);
                        if let Ok(mut guard) = inner.try_lock() {
                            guard.state = StreamerState::Error;
                        }
                    }
                }
                Ok(())
            }));
        }

        Ok(())
    }

    pub async fn pause(&mut self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.state == StreamerState::Streaming {
            if let Some(ref tx) = self.sender_tx {
                try_send_sender_message(tx, SenderMessage::Pause, "pause");
            }
            inner.state = StreamerState::Paused;
            self.state_cache
                .store(StreamerState::Paused as u8, Ordering::Relaxed);
        }
        Ok(())
    }

    pub async fn resume(&mut self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.state == StreamerState::Paused {
            if let Some(ref tx) = self.sender_tx {
                try_send_sender_message(tx, SenderMessage::Resume, "resume");
            }
            inner.state = StreamerState::Streaming;
            self.state_cache
                .store(StreamerState::Streaming as u8, Ordering::Relaxed);
        }
        Ok(())
    }

    pub async fn reset_after_flush(&mut self) {
        let mut inner = self.inner.lock().await;
        inner.first_packet_sent = false;
        for sender in &mut inner.rtp_senders {
            sender.reset_sync_state();
        }
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.state_cache
            .store(StreamerState::Stopped as u8, Ordering::Relaxed);

        if let Some(ref tx) = self.sender_tx {
            try_send_sender_message(tx, SenderMessage::Stop, "stop");
        }
        self.sender_tx = None;

        if let Some(handle) = self.sender_thread.take() {
            match tokio::task::spawn_blocking(move || handle.join()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => tracing::warn!("sender thread panicked during shutdown"),
                Err(error) => tracing::warn!(%error, "failed to join sender thread"),
            }
        }

        if let Some(task) = self.task.take() {
            task.abort();
        }

        let mut inner = self.inner.lock().await;
        inner.state = StreamerState::Stopped;
        inner.buffer.flush();
        inner.live_decoder = None;
        Ok(())
    }

    pub fn position(&self) -> u64 {
        self.timestamp_cache.load(Ordering::Relaxed)
    }

    pub async fn set_timing_offset(&mut self, offset: ClockOffset) {
        self.inner.lock().await.clock_offset = Some(offset);
    }

    pub async fn set_timing_updates(&mut self, rx: watch::Receiver<ClockOffset>) {
        self.inner.lock().await.timing_rx = Some(rx);
    }
}

fn decode_some_inner(inner: &mut StreamerInner) -> Result<()> {
    let format = inner.config.audio_format;
    let frames_per_packet = format.frames_per_packet as usize;

    for _ in 0..DECODE_BATCH_LIMIT {
        let frame = if let Some(ref mut live_decoder) = inner.live_decoder {
            live_decoder.decode_resampled(&format, frames_per_packet)?
        } else {
            break;
        };

        if let Some(frame) = frame {
            let audio_frame = crate::audio::AudioFrame::new(frame.samples);
            inner
                .buffer
                .push(audio_frame)
                .map_err(|_| crate::core::error::StreamingError::BufferOverflow)?;
        } else {
            break;
        }
    }
    Ok(())
}

fn try_send_sender_message(
    tx: &Sender<SenderMessage>,
    message: SenderMessage,
    action: &'static str,
) {
    if let Err(error) = tx.try_send(message) {
        tracing::debug!(%error, action, "failed to signal sender thread");
    }
}

enum BufferingAction {
    Ready,
    Wait,
    Stop,
}

fn update_timing_offset(inner: &mut StreamerInner) {
    if let Some(rx) = inner.timing_rx.as_mut()
        && rx.has_changed().unwrap_or(false)
    {
        let latest = *rx.borrow_and_update();
        inner.clock_offset = Some(latest);
    }
}

fn refill_buffer_if_needed(inner: &mut StreamerInner) -> Result<()> {
    if inner.buffer.fill_percentage() >= BUFFER_LOW_WATERMARK_PERCENT {
        return Ok(());
    }

    let decode_start = Instant::now();
    decode_some_inner(inner)?;
    let decode_elapsed = decode_start.elapsed();
    if decode_elapsed.as_millis() > DECODE_WARNING_THRESHOLD_MS {
        tracing::warn!(
            "Decode took {:.2}ms (blocking send loop!)",
            decode_elapsed.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

fn buffering_action(inner: &mut StreamerInner) -> BufferingAction {
    if inner.state != StreamerState::Buffering {
        return BufferingAction::Ready;
    }

    if inner.buffer.fill_percentage() > BUFFER_RESUME_FILL_PERCENT {
        inner.state = StreamerState::Streaming;
        return BufferingAction::Ready;
    }

    let live_decoder_eof = inner
        .live_decoder
        .as_ref()
        .is_none_or(|decoder| decoder.is_eof());
    let has_live_decoder = inner.live_decoder.is_some();

    if has_live_decoder && live_decoder_eof && inner.buffer.is_empty() {
        tracing::info!("Decoder EOF and buffer empty - stopping");
        inner.state = StreamerState::Stopped;
        BufferingAction::Stop
    } else {
        BufferingAction::Wait
    }
}

fn log_pcm_diag(diag: u32, samples: &[i16]) {
    if diag >= 5 && !diag.is_multiple_of(500) {
        return;
    }

    let rms: f64 = if samples.is_empty() {
        0.0
    } else {
        let sum: f64 = samples.iter().map(|&sample| (sample as f64).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    };
    let max_abs = samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    tracing::info!(
        "DIAG PCM frame #{}: samples={}, rms={:.1}, max_abs={}, first_4={:?}",
        diag,
        samples.len(),
        rms,
        max_abs,
        &samples[..samples.len().min(4)]
    );
}

fn log_encoded_diag(diag: u32, encoded: &[u8], encode_elapsed: Duration) {
    if diag >= 5 && !diag.is_multiple_of(500) {
        return;
    }

    let all_zero = encoded.iter().all(|&byte| byte == 0);
    tracing::info!(
        "DIAG ALAC packet #{}: encoded_len={}, all_zero={}, first_8={:02x?}, encode_time={:.2}ms",
        diag,
        encoded.len(),
        all_zero,
        &encoded[..encoded.len().min(8)],
        encode_elapsed.as_secs_f64() * 1000.0
    );
}

fn adjusted_wall_time(inner: &StreamerInner, local_wall: u64, diag: u32) -> u64 {
    if let Some(offset) = inner.clock_offset {
        let adjusted = inner.clock.apply_offset(local_wall, &offset);
        if diag < 3 {
            tracing::info!(
                "CLOCK OFFSET: offset_ns={}, local_wall={}, adjusted={}, diff={}",
                offset.offset_ns,
                local_wall,
                adjusted,
                adjusted as i128 - local_wall as i128
            );
        }
        adjusted
    } else {
        if diag < 3 {
            tracing::warn!(
                "CLOCK OFFSET: None - using local time directly (may cause timing issues!)"
            );
        }
        local_wall
    }
}

fn update_sent_packet_state(
    inner: &mut StreamerInner,
    rtp_ts: u32,
    first_packet: bool,
    need_sync: bool,
    next_timestamp: u64,
    timestamp_cache: &AtomicU64,
    packets_sent_counter: &AtomicU64,
) {
    if need_sync {
        inner.last_sync_rtp = rtp_ts;
    }
    if first_packet {
        inner.first_packet_sent = true;
    }
    inner.current_timestamp = next_timestamp;
    timestamp_cache.store(inner.current_timestamp, Ordering::Relaxed);
    packets_sent_counter.fetch_add(1, Ordering::Relaxed);
}

fn record_buffer_underrun(
    inner: &mut StreamerInner,
    state_cache: &AtomicU8,
    underrun_counter: &AtomicU64,
) {
    let count = underrun_counter.fetch_add(1, Ordering::Relaxed) + 1;
    if count <= 5 || count.is_multiple_of(50) {
        tracing::warn!("Buffer underrun #{} (buffer empty, packet skipped)", count);
    }
    inner.state = StreamerState::Buffering;
    state_cache.store(StreamerState::Buffering as u8, Ordering::Relaxed);
}

async fn run_streamer(
    inner: Arc<Mutex<StreamerInner>>,
    state_cache: Arc<AtomicU8>,
    timestamp_cache: Arc<AtomicU64>,
    packets_sent_counter: Arc<AtomicU64>,
    underrun_counter: Arc<AtomicU64>,
    sender_tx: Option<Sender<SenderMessage>>,
) -> Result<()> {
    let frame_duration_ns = {
        let guard = inner.lock().await;
        guard.config.audio_format.frames_per_packet as u64 * 1_000_000_000u64
            / guard.config.audio_format.sample_rate.as_hz() as u64
    };
    let frame_duration = Duration::from_nanos(frame_duration_ns);

    let has_sender_thread = sender_tx.is_some();

    if !has_sender_thread {
        set_realtime_priority();
    }

    let mut next_deadline = Instant::now();

    loop {
        {
            let state = inner.lock().await.state;
            if state == StreamerState::Stopped || state == StreamerState::Error {
                break;
            }
            if state == StreamerState::Paused {
                sleep(BUFFER_RETRY_SLEEP).await;
                next_deadline = Instant::now();
                continue;
            }
        }

        {
            let mut guard = inner.lock().await;
            update_timing_offset(&mut guard);
            refill_buffer_if_needed(&mut guard)?;

            match buffering_action(&mut guard) {
                BufferingAction::Ready => {
                    if guard.state == StreamerState::Streaming {
                        state_cache.store(StreamerState::Streaming as u8, Ordering::Relaxed);
                    }
                }
                BufferingAction::Stop => {
                    state_cache.store(StreamerState::Stopped as u8, Ordering::Relaxed);
                    break;
                }
                BufferingAction::Wait => {
                    state_cache.store(StreamerState::Buffering as u8, Ordering::Relaxed);
                    drop(guard);
                    sleep(BUFFER_RETRY_SLEEP).await;
                    next_deadline = Instant::now();
                    continue;
                }
            }

            let frame = guard.buffer.pop();
            if let Some(frame) = frame {
                static DIAG_COUNT: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                let diag = DIAG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                log_pcm_diag(diag, &frame.samples);

                let encode_start = Instant::now();
                let encoder = guard.encoder.as_mut().ok_or_else(|| {
                    crate::core::error::StreamingError::Encoding("Encoder missing".into())
                })?;
                let packet = encoder.encode(&frame.samples)?;
                let encode_elapsed = encode_start.elapsed();
                log_encoded_diag(diag, &packet.data, encode_elapsed);

                let payload_type = guard.config.stream_type as u8;
                let sample_rate = guard.config.audio_format.sample_rate.as_hz();
                let last_sync_rtp = guard.last_sync_rtp;

                let local_wall = guard.clock.now_wall_ns();
                let adjusted = adjusted_wall_time(&guard, local_wall, diag);

                let render_adjusted = adjusted + guard.render_delay_ns;
                let ntp = unix_to_ntp(render_adjusted);

                let first_packet = !guard.first_packet_sent;
                let marker = first_packet;
                if marker {
                    tracing::info!("Sending first audio packet with marker bit set");
                }

                let rtp_ts = packet.timestamp as u32;

                let need_sync = first_packet
                    || last_sync_rtp == 0
                    || rtp_ts.wrapping_sub(last_sync_rtp) >= sample_rate;

                let use_ptp_sync = guard.use_ptp_sync;
                let ptp_clock_id = guard.ptp_master_clock_id;

                if !guard.rtp_senders.is_empty() {
                    if let Some(ref tx) = sender_tx {
                        let sync_data = if need_sync {
                            if use_ptp_sync {
                                let next_rtp_ts =
                                    next_airplay_packet_timestamp(rtp_ts, sample_rate);
                                guard.rtp_senders[0].prepare_ptp_sync(
                                    rtp_ts,
                                    render_adjusted,
                                    next_rtp_ts,
                                    &ptp_clock_id,
                                )?
                            } else {
                                guard.rtp_senders[0].prepare_sync(rtp_ts, ntp)?
                            }
                        } else {
                            None
                        };

                        let mut wire_packets = Vec::with_capacity(guard.rtp_senders.len());
                        for sender in &mut guard.rtp_senders {
                            wire_packets.push(sender.prepare_audio(
                                payload_type,
                                rtp_ts,
                                &packet.data,
                                marker,
                            )?);
                        }

                        if diag < 5 || diag.is_multiple_of(500) {
                            tracing::info!(
                                "DIAG timing #{}: encode={:.2}ms, targets={} (sender thread handles send timing)",
                                diag,
                                encode_elapsed.as_secs_f64() * 1000.0,
                                wire_packets.len(),
                            );
                        }

                        update_sent_packet_state(
                            &mut guard,
                            rtp_ts,
                            first_packet,
                            need_sync,
                            packet.timestamp + packet.samples as u64,
                            &timestamp_cache,
                            &packets_sent_counter,
                        );

                        drop(guard);
                        let tx_clone = tx.clone();
                        let msg = SenderMessage::Packet {
                            wire_packets,
                            sync_data,
                        };
                        let send_result =
                            tokio::task::spawn_blocking(move || tx_clone.send(msg)).await;
                        match send_result {
                            Ok(Ok(())) => {}
                            _ => {
                                tracing::error!("Sender thread disconnected");
                                state_cache.store(StreamerState::Error as u8, Ordering::Relaxed);
                                break;
                            }
                        }
                        continue;
                    } else {
                        if need_sync {
                            if use_ptp_sync {
                                let next_rtp_ts =
                                    next_airplay_packet_timestamp(rtp_ts, sample_rate);
                                guard.rtp_senders[0].send_ptp_sync(
                                    rtp_ts,
                                    render_adjusted,
                                    next_rtp_ts,
                                    &ptp_clock_id,
                                )?;
                            } else {
                                guard.rtp_senders[0].send_sync(rtp_ts, ntp)?;
                            }
                        }

                        let send_start = Instant::now();
                        for sender in &mut guard.rtp_senders {
                            sender.send_audio(payload_type, rtp_ts, &packet.data, marker)?;
                        }
                        let send_elapsed = send_start.elapsed();

                        if diag < 5 || diag.is_multiple_of(500) {
                            tracing::info!(
                                "DIAG timing #{}: encode={:.2}ms, send={:.2}ms, targets={}, total={:.2}ms",
                                diag,
                                encode_elapsed.as_secs_f64() * 1000.0,
                                send_elapsed.as_secs_f64() * 1000.0,
                                guard.rtp_senders.len(),
                                (encode_elapsed + send_elapsed).as_secs_f64() * 1000.0
                            );
                        }

                        update_sent_packet_state(
                            &mut guard,
                            rtp_ts,
                            first_packet,
                            need_sync,
                            packet.timestamp + packet.samples as u64,
                            &timestamp_cache,
                            &packets_sent_counter,
                        );
                    }
                }
            } else {
                record_buffer_underrun(&mut guard, &state_cache, &underrun_counter);
            }
        }

        if has_sender_thread {
            //
            tokio::task::yield_now().await;
        } else {
            next_deadline += frame_duration;
            let now = Instant::now();
            if next_deadline > now {
                tokio::time::sleep(next_deadline - now).await;
            }
        }
    }

    if let Some(ref tx) = sender_tx {
        try_send_sender_message(tx, SenderMessage::Stop, "stop");
    }

    Ok(())
}
