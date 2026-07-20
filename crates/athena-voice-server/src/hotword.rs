//! Hotword detection (stub).

/// Hotword detector (stub).
#[derive(Debug)]
pub struct HotwordDetector;

impl HotwordDetector {
    /// Load the model (stub).
    pub fn load(_model_dir: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self)
    }

    /// Check for hotword (stub).
    pub fn detect(&self, _audio: &[i16]) -> bool {
        true
    }

    /// Sample rate (stub).
    pub fn sample_rate(&self) -> u32 { 16000 }
}