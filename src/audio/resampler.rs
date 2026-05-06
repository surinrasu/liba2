use crate::core::error::Result;
use rubato::{
    Resampler as RubatoResampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use tracing::{debug, info};

pub const DEFAULT_CHUNK_SIZE: usize = 1024;

const SINC_PARAMS: SincInterpolationParameters = SincInterpolationParameters {
    sinc_len: 512,
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Cubic,
    oversampling_factor: 256,
    window: WindowFunction::BlackmanHarris2,
};

pub struct Resampler {
    inner: SincFixedIn<f32>,
    channels: usize,
}

impl Resampler {
    pub fn new(source_rate: u32, target_rate: u32, channels: u8) -> Result<Self> {
        Self::with_chunk_size(source_rate, target_rate, channels, DEFAULT_CHUNK_SIZE)
    }

    pub fn with_chunk_size(
        source_rate: u32,
        target_rate: u32,
        channels: u8,
        chunk_size: usize,
    ) -> Result<Self> {
        let resample_ratio = target_rate as f64 / source_rate as f64;
        let channels_usize = channels as usize;

        let mut inner = SincFixedIn::<f32>::new(
            resample_ratio,
            2.0, // Max relative ratio deviation
            SINC_PARAMS,
            chunk_size,
            channels_usize,
        )
        .map_err(|e| {
            crate::core::error::StreamingError::Encoding(format!(
                "Failed to create resampler: {}",
                e
            ))
        })?;

        let output_delay = inner.output_delay();
        info!(
            "Resampler created: {}Hz -> {}Hz, {} channels, delay: {} frames ({:.1}ms)",
            source_rate,
            target_rate,
            channels,
            output_delay,
            output_delay as f32 / target_rate as f32 * 1000.0
        );

        let input_frames_needed = inner.input_frames_next();
        let priming_chunks = (SINC_PARAMS.sinc_len / input_frames_needed).max(3);

        let silence_chunk: Vec<Vec<f32>> = (0..channels_usize)
            .map(|_| vec![0.0f32; input_frames_needed])
            .collect();

        for i in 0..priming_chunks {
            match inner.process(&silence_chunk, None) {
                Ok(_) => {
                    debug!("Priming chunk {}/{} processed", i + 1, priming_chunks);
                }
                Err(e) => {
                    debug!("Error during resampler priming: {}", e);
                    break;
                }
            }
        }
        info!("Resampler primed with {} chunks of silence", priming_chunks);

        Ok(Self {
            inner,
            channels: channels_usize,
        })
    }

    pub fn process(&mut self, samples: &[i16]) -> Result<Vec<i16>> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }

        let num_frames = samples.len() / self.channels;
        let mut input_channels: Vec<Vec<f32>> = (0..self.channels)
            .map(|_| Vec::with_capacity(num_frames))
            .collect();

        for frame_idx in 0..num_frames {
            for ch in 0..self.channels {
                let sample = samples[frame_idx * self.channels + ch];
                input_channels[ch].push(sample as f32 / 32768.0);
            }
        }

        let resampled = self.process_f32(&input_channels)?;

        Ok(interleave_with_dither(&resampled))
    }

    pub fn process_f32(&mut self, channels: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        if channels.is_empty() || channels[0].is_empty() {
            return Ok(Vec::new());
        }

        self.inner.process(channels, None).map_err(|e| {
            crate::core::error::StreamingError::Encoding(format!("Resampling error: {}", e)).into()
        })
    }

    pub fn input_frames_next(&self) -> usize {
        self.inner.input_frames_next()
    }
}

#[inline]
pub fn dither_to_i16(sample: f32) -> i16 {
    let rand1 = fastrand::f32() - 0.5; // Uniform -0.5 to 0.5
    let rand2 = fastrand::f32() - 0.5;
    let tpdf_noise = (rand1 + rand2) / 32768.0; // Triangular, scaled to 1 LSB

    let dithered = sample + tpdf_noise;
    (dithered * 32767.0).clamp(-32768.0, 32767.0) as i16
}

pub fn interleave_with_dither(channels: &[Vec<f32>]) -> Vec<i16> {
    if channels.is_empty() || channels[0].is_empty() {
        return Vec::new();
    }

    let num_frames = channels[0].len();
    let num_channels = channels.len();
    let mut output = Vec::with_capacity(num_frames * num_channels);

    for frame_idx in 0..num_frames {
        for channel in channels.iter().take(num_channels) {
            output.push(dither_to_i16(channel[frame_idx]));
        }
    }

    output
}
