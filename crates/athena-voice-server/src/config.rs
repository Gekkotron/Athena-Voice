//! Runtime configuration.

use std::path::PathBuf;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the directory containing model files.
    pub model_dir: PathBuf,
    /// Socket path for audio input.
    pub audio_socket: PathBuf,
    /// Socket path for event output.
    pub event_socket: PathBuf,
    /// VAD aggressiveness (0-3).
    pub vad_aggressiveness: u8,
    /// ASR model name.
    pub asr_model: String,
    /// TTS model name.
    pub tts_model: String,
    /// TTS voice.
    pub tts_voice: String,
    /// TTS sample rate.
    pub tts_sample_rate: u32,
}
