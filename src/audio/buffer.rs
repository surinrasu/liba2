use crate::core::AudioFormat;
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub samples: Arc<Vec<i16>>,
}

impl AudioFrame {
    pub fn new(samples: Vec<i16>) -> Self {
        Self {
            samples: Arc::new(samples),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferError {
    Overflow,
}

pub struct AudioBuffer {
    frames: VecDeque<AudioFrame>,
    format: AudioFormat,
    capacity_frames: usize,
    total_samples_written: u64,
    total_samples_read: u64,
}

impl AudioBuffer {
    pub fn new(format: AudioFormat, capacity_ms: u32) -> Self {
        let sample_rate = format.sample_rate.as_hz();
        let samples_per_ms = sample_rate / 1000;
        let capacity_samples = samples_per_ms * capacity_ms;
        let capacity_frames = capacity_samples
            .checked_div(format.frames_per_packet)
            .unwrap_or(1) as usize;

        Self {
            frames: VecDeque::with_capacity(capacity_frames.max(1)),
            format,
            capacity_frames: capacity_frames.max(1),
            total_samples_written: 0,
            total_samples_read: 0,
        }
    }

    pub fn push(&mut self, frame: AudioFrame) -> Result<(), BufferError> {
        if self.frames.len() >= self.capacity_frames {
            return Err(BufferError::Overflow);
        }

        let sample_count = frame.samples.len() as u64 / self.format.channels as u64;
        self.total_samples_written += sample_count;
        self.frames.push_back(frame);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<AudioFrame> {
        let frame = self.frames.pop_front()?;
        let sample_count = frame.samples.len() as u64 / self.format.channels as u64;
        self.total_samples_read += sample_count;
        Some(frame)
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn fill_percentage(&self) -> f32 {
        if self.capacity_frames == 0 {
            return 0.0;
        }
        (self.frames.len() as f32 / self.capacity_frames as f32) * 100.0
    }

    pub fn flush(&mut self) -> Vec<AudioFrame> {
        self.frames.drain(..).collect()
    }
}
