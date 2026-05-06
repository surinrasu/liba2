#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Buffering,
    Error,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlaybackInfo {
    pub state: PlaybackState,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f32,
    pub buffer_level: f32,
    pub is_muted: bool,
}

impl Default for PlaybackInfo {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            position: 0.0,
            duration: None,
            volume: 1.0,
            buffer_level: 0.0,
            is_muted: false,
        }
    }
}

impl PlaybackInfo {
    pub fn remaining(&self) -> Option<f64> {
        self.duration.map(|d| d - self.position)
    }

    pub fn progress_percentage(&self) -> Option<f32> {
        self.duration.map(|d| (self.position / d * 100.0) as f32)
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            PlaybackState::Playing | PlaybackState::Buffering
        )
    }
}
