//! Whisper.cpp ASR (stub).

/// Whisper ASR model (stub).
#[derive(Debug)]
pub struct WhisperAsr;

impl WhisperAsr {
    /// Load the model.
    pub fn load(_model_path: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self)
    }

    /// Transcribe audio (stub).
    pub fn transcribe(&self, _audio: &[i16]) -> anyhow::Result<String> {
        Ok("Athéna detected".to_string())
    }
}
