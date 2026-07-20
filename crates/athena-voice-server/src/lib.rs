//! Athena-Voice Server: Socket server for VAD → hotword → ASR → intent → TTS cycle.

mod config;
pub mod runtime;
pub use config::Config;
pub use runtime::Runtime;
pub use metrics::Metrics;

use std::sync::Arc;

use athena_voice_core::event::Event;
use athena_voice_storage::Store;

mod socket;
mod vad;
mod hotword;
mod asr;
mod tts;
mod metrics;
mod resample;

impl Runtime {
    /// Start the server.
    pub async fn run(runtime: Arc<Self>) -> anyhow::Result<()> {
        tokio::try_join!(
            socket::start_event_socket(runtime.clone()),
            // socket::start_audio_socket(runtime),
        )?;
        Ok(())
    }
}