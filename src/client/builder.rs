use crate::client::DEFAULT_RENDER_DELAY_MS;
use crate::core::{AudioFormat, StreamConfig};
use crate::{AirPlayClient, Result};

pub struct ClientBuilder {
    stream_config: StreamConfig,
    render_delay_ms: u32,
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self {
            stream_config: StreamConfig::default(),
            render_delay_ms: DEFAULT_RENDER_DELAY_MS,
        }
    }

    pub fn stream_config(mut self, config: StreamConfig) -> Self {
        self.stream_config = config;
        self
    }

    pub fn audio_format(mut self, format: AudioFormat) -> Self {
        self.stream_config.audio_format = format;
        self
    }

    pub fn airplay2_buffered(mut self) -> Self {
        self.stream_config = StreamConfig::airplay2_buffered();
        self
    }

    pub fn airplay1(mut self) -> Self {
        self.stream_config = StreamConfig::airplay1_realtime();
        self
    }

    pub fn render_delay_ms(mut self, ms: u32) -> Self {
        self.render_delay_ms = ms;
        self
    }

    pub fn build(self) -> Result<AirPlayClient> {
        let mut client = AirPlayClient::with_config(self.stream_config)?;
        client.set_render_delay_ms(self.render_delay_ms);
        Ok(client)
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
