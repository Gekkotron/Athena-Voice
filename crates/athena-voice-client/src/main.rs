//! Athena-Voice Client: CLI for audio playback and socket interaction.

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Args {
    #[clap(long, default_value = "/tmp/athena-audio.sock")]
    audio_socket: String,

    #[clap(long)]
    wav_file: Option<String>,

    #[clap(long)]
    serial_port: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    if let Some(wav_path) = args.wav_file {
        play_wav(&wav_path, &args.audio_socket).await?;
    } else if let Some(port) = args.serial_port {
        play_serial(&port, &args.audio_socket).await?;
    } else {
        record_mic(&args.audio_socket).await?;
    }

    Ok(())
}

async fn play_wav(_wav_path: &str, _socket_path: &str) -> anyhow::Result<()> {
    unimplemented!("WAV playback")
}

async fn play_serial(_port: &str, _socket_path: &str) -> anyhow::Result<()> {
    unimplemented!("Serial playback")
}

async fn record_mic(_socket_path: &str) -> anyhow::Result<()> {
    unimplemented!("Microphone recording")
}
