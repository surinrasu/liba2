use crate::core::{AudioFormat, error::Result};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LivePcmFrame {
    pub samples: Vec<i16>,
    pub channels: u8,
    pub sample_rate: u32,
}

impl LivePcmFrame {
    pub fn new(samples: Vec<i16>, channels: u8, sample_rate: u32) -> Self {
        Self {
            samples,
            channels,
            sample_rate,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LiveAudioSendError {
    #[error("live audio queue is full")]
    Full,

    #[error("live audio queue is disconnected")]
    Disconnected,
}

pub struct LiveFrameSender {
    tx: Sender<LivePcmFrame>,
}

impl LiveFrameSender {
    pub fn try_send(&self, frame: LivePcmFrame) -> std::result::Result<(), LiveAudioSendError> {
        match self.tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                tracing::debug!("Live audio channel full, dropping frame");
                Err(LiveAudioSendError::Full)
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::debug!("Live audio channel disconnected");
                Err(LiveAudioSendError::Disconnected)
            }
        }
    }

    pub fn send(&self, frame: LivePcmFrame) -> std::result::Result<(), LiveAudioSendError> {
        self.tx
            .send(frame)
            .map_err(|_| LiveAudioSendError::Disconnected)
    }

    pub fn capacity(&self) -> Option<usize> {
        self.tx.capacity()
    }

    pub fn is_full(&self) -> bool {
        self.tx.is_full()
    }
}

pub struct LiveAudioDecoder {
    rx: Receiver<LivePcmFrame>,
    sample_rate: u32,
    channels: u8,
    position_samples: u64,
    eof: bool,
    residual_samples: Vec<i16>,
    recv_timeout: Duration,
    resampler: Option<crate::audio::resampler::Resampler>,
}

impl LiveAudioDecoder {
    pub fn new(rx: Receiver<LivePcmFrame>, sample_rate: u32, channels: u8) -> Self {
        Self {
            rx,
            sample_rate,
            channels,
            position_samples: 0,
            eof: false,
            residual_samples: Vec::new(),
            recv_timeout: Duration::from_millis(2), // Very short timeout to prevent blocking!
            resampler: None,
        }
    }

    pub fn create_pair(sample_rate: u32, channels: u8, capacity: usize) -> (LiveFrameSender, Self) {
        let (tx, rx) = bounded::<LivePcmFrame>(capacity);
        let sender = LiveFrameSender { tx };
        let decoder = Self::new(rx, sample_rate, channels);
        (sender, decoder)
    }

    pub fn decode_frame(&mut self) -> Result<Option<DecodedFrame>> {
        if self.eof {
            return Ok(None);
        }

        match self.rx.recv_timeout(self.recv_timeout) {
            Ok(frame) => {
                let num_frames = frame.samples.len() / frame.channels as usize;
                let decoded = DecodedFrame {
                    samples: frame.samples,
                };
                self.position_samples += num_frames as u64;
                Ok(Some(decoded))
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                tracing::trace!("Live decoder: receive timeout (no data)");
                Ok(None)
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                tracing::debug!("Live decoder: channel disconnected, marking EOF");
                self.eof = true;
                Ok(None)
            }
        }
    }

    pub fn decode_resampled(
        &mut self,
        target_format: &AudioFormat,
        frames_per_packet: usize,
    ) -> Result<Option<DecodedFrame>> {
        let target_rate = target_format.sample_rate.as_hz();
        let source_rate = self.sample_rate;

        if source_rate != target_rate && self.resampler.is_none() {
            self.resampler = Some(crate::audio::resampler::Resampler::new(
                source_rate,
                target_rate,
                self.channels,
            )?);
        }

        let mut collected_samples = std::mem::take(&mut self.residual_samples);
        let target_samples = frames_per_packet * target_format.channels as usize;

        while collected_samples.len() < target_samples {
            match self.decode_frame()? {
                Some(frame) => {
                    if source_rate == target_rate {
                        collected_samples.extend(frame.samples);
                    } else {
                        let resampled = self
                            .resampler
                            .as_mut()
                            .expect("resampler should be initialized")
                            .process(&frame.samples)?;
                        collected_samples.extend(resampled);
                    }
                }
                None => {
                    if collected_samples.is_empty() {
                        return Ok(None);
                    }
                    self.residual_samples = collected_samples;
                    return Ok(None);
                }
            }
        }

        if collected_samples.is_empty() {
            return Ok(None);
        }

        if collected_samples.len() > target_samples {
            self.residual_samples = collected_samples[target_samples..].to_vec();
            collected_samples.truncate(target_samples);
        }

        Ok(Some(DecodedFrame {
            samples: collected_samples,
        }))
    }

    pub fn is_eof(&self) -> bool {
        self.eof && self.rx.is_empty()
    }
}
