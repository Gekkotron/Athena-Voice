//! Athena-Voice Server: Socket server for VAD → hotword → ASR → intent → TTS cycle.

use std::sync::Arc;

use athena_voice_server::{Config, Runtime};
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Args {
    #[clap(long, default_value = "/var/lib/athena/models")]
    model_dir: String,

    #[clap(long, default_value = "/tmp/athena-audio.sock")]
    audio_socket: String,

    #[clap(long, default_value = "/tmp/athena-events.sock")]
    event_socket: String,

    #[clap(long, default_value = "2")]
    vad_aggressiveness: u8,

    #[clap(long, default_value = "ggml-small")]
    asr_model: String,

    #[clap(long, default_value = "piper-fr")]
    tts_model: String,

    #[clap(long, default_value = "bl_lightspeed")]
    tts_voice: String,

    #[clap(long, default_value = "22050")]
    tts_sample_rate: u32,

    #[clap(long, default_value = "data/athena.db")]
    sqlite_path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let store = Arc::new(athena_voice_storage::InMemoryStore::default());
    let config = Config {
        model_dir: args.model_dir.into(),
        audio_socket: args.audio_socket.into(),
        event_socket: args.event_socket.into(),
        vad_aggressiveness: args.vad_aggressiveness.clamp(0, 3),
        asr_model: args.asr_model,
        tts_model: args.tts_model,
        tts_voice: args.tts_voice,
        tts_sample_rate: args.tts_sample_rate,
    };

    let runtime = Arc::new(Runtime::new(config, store).await?);
    Runtime::run(runtime).await
}
