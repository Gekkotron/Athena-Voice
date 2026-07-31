//! Athena-Voice TTS worker — bridges the generic MQTT TTS protocol to a
//! local speech synthesizer.
//!
//! Wire protocol (see `athena-voice-providers/src/remote/mqtt_tts.rs`):
//! - requests arrive on `athena/providers/tts/<name>/request` as JSON
//!   `{ "session_id": "<uuid>", "locale": "fr", "text": "..." }`
//! - responses go to `athena/providers/tts/<name>/response` as JSON
//!   `{ "session_id", "chunk_b64", "done" }` — s16le mono PCM chunks,
//!   base64-encoded, terminated by a `done: true` marker.
//!
//! The synthesis engine is macOS `say` (output converted to WAV via the
//! bundled `afconvert`). Swap `synthesize_wav` for a Piper invocation to get
//! a portable worker; the wire protocol stays identical.

use std::process::Command;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use clap::Parser;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tracing::{error, info, warn};

#[derive(Debug, Parser)]
#[command(about = "MQTT TTS worker backed by the OS speech synthesizer")]
struct Args {
    /// MQTT broker host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// MQTT broker port.
    #[arg(long, default_value_t = 1883)]
    port: u16,

    /// Provider name — must match `tts = { mqtt_tts = { name = "..." } }`.
    #[arg(long, default_value = "say")]
    name: String,

    /// Voice passed to `say -v` for French locales.
    #[arg(long, default_value = "Thomas")]
    voice: String,

    /// Output sample rate in Hz (s16le mono).
    #[arg(long, default_value_t = 22_050)]
    rate: u32,

    /// Chunk size in milliseconds.
    #[arg(long, default_value_t = 200)]
    chunk_ms: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    anyhow::ensure!(
        Command::new("say").arg("--version").output().is_ok(),
        "`say` not found — this worker currently requires macOS"
    );

    let request_topic = format!("athena/providers/tts/{}/request", args.name);
    let response_topic = format!("athena/providers/tts/{}/response", args.name);

    let mut opts = MqttOptions::new(
        format!("athena-tts-worker-{}-{}", args.name, std::process::id()),
        &args.host,
        args.port,
    );
    opts.set_keep_alive(Duration::from_secs(15));
    // Synthesized chunks for a long sentence can exceed the 10 KiB default.
    opts.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    client.subscribe(&request_topic, QoS::AtLeastOnce).await?;

    info!(topic = %request_topic, voice = %args.voice, rate = args.rate, "TTS worker ready");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == request_topic => {
                let client = client.clone();
                let response_topic = response_topic.clone();
                let voice = args.voice.clone();
                let (rate, chunk_ms) = (args.rate, args.chunk_ms);
                let payload = p.payload.to_vec();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_request(&client, &response_topic, &payload, &voice, rate, chunk_ms)
                            .await
                    {
                        error!(error = %e, "TTS request failed");
                    }
                });
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, "mqtt error; retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

async fn handle_request(
    client: &AsyncClient,
    response_topic: &str,
    payload: &[u8],
    voice: &str,
    rate: u32,
    chunk_ms: u32,
) -> anyhow::Result<()> {
    let request: serde_json::Value = serde_json::from_slice(payload)?;
    let session_id = request
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("request missing session_id"))?
        .to_string();
    let text = request
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let locale = request
        .get("locale")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("fr")
        .to_string();

    info!(session = %session_id, text = %text, "synthesizing");

    let samples = {
        let voice = voice.to_string();
        tokio::task::spawn_blocking(move || synthesize_wav(&text, &locale, &voice, rate)).await??
    };

    // Stream fixed-duration chunks, then the done marker.
    let samples_per_chunk = (rate * chunk_ms / 1000).max(1) as usize;
    for chunk in samples.chunks(samples_per_chunk) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let msg = serde_json::json!({
            "session_id": session_id,
            "chunk_b64": STANDARD.encode(&bytes),
            "done": false,
        });
        client
            .publish(response_topic, QoS::AtLeastOnce, false, msg.to_string())
            .await?;
    }
    let done = serde_json::json!({ "session_id": session_id, "done": true });
    client
        .publish(response_topic, QoS::AtLeastOnce, false, done.to_string())
        .await?;
    Ok(())
}

/// Synthesizes `text` to s16le mono samples at `rate` via `say` + `afconvert`.
fn synthesize_wav(text: &str, locale: &str, voice: &str, rate: u32) -> anyhow::Result<Vec<i16>> {
    let dir = tempfile::tempdir()?;
    let aiff = dir.path().join("out.aiff");
    let wav = dir.path().join("out.wav");

    let mut say = Command::new("say");
    if locale.starts_with("fr") {
        say.args(["-v", voice]);
    }
    let status = say.arg("-o").arg(&aiff).arg(text).status()?;
    // Unknown voice: retry with the system default rather than failing.
    if !status.success() {
        let status = Command::new("say")
            .arg("-o")
            .arg(&aiff)
            .arg(text)
            .status()?;
        anyhow::ensure!(status.success(), "say failed");
    }

    let status = Command::new("afconvert")
        .args(["-f", "WAVE", "-d"])
        .arg(format!("LEI16@{rate}"))
        .args(["-c", "1"])
        .arg(&aiff)
        .arg(&wav)
        .status()?;
    anyhow::ensure!(status.success(), "afconvert failed");

    let mut reader = hound::WavReader::open(&wav)?;
    let samples: Result<Vec<i16>, _> = reader.samples::<i16>().collect();
    Ok(samples?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn chunking_math_never_zero() {
        // rate * chunk_ms / 1000 could truncate to 0 for tiny values; the
        // worker clamps to 1 so `chunks()` can't panic.
        let samples_per_chunk = (8u32 * 10 / 1000).max(1) as usize;
        assert_eq!(samples_per_chunk, 1);
    }
}
