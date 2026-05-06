use crate::core::{AudioCodec, AudioFormat, error::Result};

#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub samples: u32,
    pub timestamp: u64,
}

pub trait AudioEncoder: Send {
    fn encode(&mut self, samples: &[i16]) -> Result<EncodedPacket>;
}

pub struct AlacEncoder {
    encoder: alac_encoder::AlacEncoder,
    input_format: alac_encoder::FormatDescription,
    format: AudioFormat,
    timestamp: u64,
    buffer: Vec<i16>,
    output_buffer: Vec<u8>,
}

impl AlacEncoder {
    pub fn new(format: AudioFormat) -> Result<Self> {
        let alac_format = alac_encoder::FormatDescription::alac(
            format.sample_rate.as_hz() as f64,
            format.frames_per_packet,
            format.channels as u32,
        );

        let pcm_format = alac_encoder::FormatDescription::pcm::<i16>(
            format.sample_rate.as_hz() as f64,
            format.channels as u32,
        );

        let encoder = alac_encoder::AlacEncoder::new(&alac_format);

        let max_encoded_size =
            format.frames_per_packet as usize * format.channels as usize * 2 + 256;

        Ok(Self {
            encoder,
            input_format: pcm_format,
            format,
            timestamp: 0,
            buffer: Vec::new(),
            output_buffer: vec![0u8; max_encoded_size],
        })
    }

    pub fn magic_cookie(&self) -> Vec<u8> {
        self.encoder.magic_cookie().to_vec()
    }

    fn encode_frame(&mut self, samples: &[i16]) -> Result<EncodedPacket> {
        let num_samples = samples.len() / self.format.channels as usize;

        let input_bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_ne_bytes()).collect();

        let encoded_size =
            self.encoder
                .encode(&self.input_format, &input_bytes, &mut self.output_buffer);

        let timestamp = self.timestamp;
        self.timestamp += num_samples as u64;

        Ok(EncodedPacket {
            data: self.output_buffer[..encoded_size].to_vec(),
            samples: num_samples as u32,
            timestamp,
        })
    }
}

impl AudioEncoder for AlacEncoder {
    fn encode(&mut self, samples: &[i16]) -> Result<EncodedPacket> {
        let samples_per_frame =
            self.format.frames_per_packet as usize * self.format.channels as usize;

        self.buffer.extend_from_slice(samples);

        if self.buffer.len() >= samples_per_frame {
            let frame_samples: Vec<i16> = self.buffer.drain(..samples_per_frame).collect();
            self.encode_frame(&frame_samples)
        } else {
            let partial: Vec<i16> = self.buffer.drain(..).collect();
            self.encode_frame(&partial)
        }
    }
}

pub fn create_encoder(format: AudioFormat) -> Result<Box<dyn AudioEncoder>> {
    match format.codec {
        AudioCodec::Alac => Ok(Box::new(AlacEncoder::new(format)?)),
        _ => Err(crate::core::error::StreamingError::InvalidFormat(format!(
            "Unsupported codec: {:?}",
            format.codec
        ))
        .into()),
    }
}
