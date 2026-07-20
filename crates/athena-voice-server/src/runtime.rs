//! Server runtime.

use std::path::PathBuf;
use std::sync::Arc;

use athena_voice_core::event::Event;
use athena_voice_storage::Store;
use tokio::sync::mpsc;

use crate::asr::WhisperAsr;
use crate::config::Config;
use crate::hotword::HotwordDetector;
use crate::metrics::Metrics;
use crate::tts::PiperTts;
use crate::vad::VadDetector;

/// Server runtime state.
pub struct Runtime {
    pub config: Config,
    pub store: Arc<dyn Store>,
    pub event_tx: mpsc::Sender<Event>,
    pub metrics: Metrics,
    pub vad: VadDetector,
    pub hotword: HotwordDetector,
    pub asr: WhisperAsr,
    pub tts: PiperTts,
}

impl Runtime {
    /// Create a new runtime.
    pub async fn new(config: Config, store: Arc<dyn Store>) -> anyhow::Result<Self> {
        let (event_tx, _) = mpsc::channel(32);
        let vad = VadDetector::new(config.vad_aggressiveness)?;
        let hotword = HotwordDetector::load(&config.model_dir)?;
        let model_path = config
            .model_dir
            .join(&config.asr_model)
            .with_extension("bin");
        let asr = WhisperAsr::load(&model_path)?;
        let tts = PiperTts::load(&config.model_dir, &config.tts_model, &config.tts_voice)?;

        Ok(Self {
            config,
            store,
            event_tx,
            metrics: Metrics::default(),
            vad,
            hotword,
            asr,
            tts,
        })
    }

    /// Access the runtime config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Metrics.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }
}
