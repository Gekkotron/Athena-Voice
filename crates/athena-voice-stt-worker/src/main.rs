//! Athena-Voice STT worker — bridges the generic MQTT STT protocol to
//! whisper.cpp.
//!
//! Wire protocol (see `athena-voice-providers/src/remote/mqtt_stt.rs`):
//! - `{session_id, locale, format, sample_rate}` opens a session buffer
//! - `{session_id, audio_b64}` appends s16le PCM to it
//! - `{session_id, done: true}` is an utterance boundary: transcribe the
//!   buffer, publish `{session_id, text, is_final: true}` on the response
//!   topic, clear the buffer, and keep the session for further audio
//!
//! Engine: the `whisper-cli` binary from the repo's whisper.cpp submodule
//! (build with `cmake -B build && cmake --build build --target whisper-cli`
//! inside `whisper.cpp/`). The model must be a real ggml file — the ones
//! committed under `models/` are placeholders; `ggml-small` is the smallest
//! that transcribes French reliably.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use clap::Parser;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use tracing::{error, info, warn};

/// Session buffers older than this are dropped (satellite died mid-utterance).
const SESSION_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Parser)]
#[command(about = "MQTT STT worker backed by whisper.cpp")]
struct Args {
    /// MQTT broker host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// MQTT broker port.
    #[arg(long, default_value_t = 1883)]
    port: u16,

    /// Provider name — must match `stt = { mqtt_stt = { name = "..." } }`.
    #[arg(long, default_value = "whisper")]
    name: String,

    /// Path to the whisper-cli binary.
    #[arg(long, default_value = "./whisper.cpp/build/bin/whisper-cli")]
    whisper_bin: PathBuf,

    /// Path to a real ggml model file.
    #[arg(long, default_value = "./models/ggml-small.bin")]
    model: PathBuf,

    /// Sample rate of incoming PCM (must match the provider contract).
    #[arg(long, default_value_t = 16_000)]
    rate: u32,
}

struct SessionBuf {
    pcm: Vec<u8>,
    locale: String,
    updated: Instant,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    anyhow::ensure!(
        args.whisper_bin.exists(),
        "whisper-cli not found at {} — build it: cd whisper.cpp && cmake -B build && cmake --build build --target whisper-cli",
        args.whisper_bin.display()
    );
    let model_size = std::fs::metadata(&args.model).map(|m| m.len()).unwrap_or(0);
    anyhow::ensure!(
        model_size > 1_000_000,
        "{} is not a real ggml model ({} bytes — the files committed under models/ are placeholders); \
         download one, e.g.: curl -L -o models/ggml-small.bin \
         https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        args.model.display(),
        model_size
    );

    let request_topic = format!("athena/providers/stt/{}/request", args.name);
    let response_topic = format!("athena/providers/stt/{}/response", args.name);

    let mut opts = MqttOptions::new(
        format!("athena-stt-worker-{}-{}", args.name, std::process::id()),
        &args.host,
        args.port,
    );
    opts.set_keep_alive(Duration::from_secs(15));
    // Audio messages carry base64 PCM — far above the 10 KiB default cap.
    opts.set_max_packet_size(8 * 1024 * 1024, 8 * 1024 * 1024);
    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    client.subscribe(&request_topic, QoS::AtLeastOnce).await?;

    info!(
        topic = %request_topic,
        model = %args.model.display(),
        "STT worker ready"
    );

    let mut sessions: HashMap<String, SessionBuf> = HashMap::new();

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == request_topic => {
                handle_message(&client, &response_topic, &p.payload, &mut sessions, &args).await;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, "mqtt error; retrying");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

async fn handle_message(
    client: &AsyncClient,
    response_topic: &str,
    payload: &[u8],
    sessions: &mut HashMap<String, SessionBuf>,
    args: &Args,
) {
    sessions.retain(|_, s| s.updated.elapsed() < SESSION_TTL);

    let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return;
    };
    let Some(sid) = v.get("session_id").and_then(serde_json::Value::as_str) else {
        return;
    };
    let sid = sid.to_string();

    if let Some(b64) = v.get("audio_b64").and_then(serde_json::Value::as_str) {
        if let Ok(bytes) = STANDARD.decode(b64) {
            sessions
                .entry(sid)
                .or_insert_with(|| SessionBuf {
                    pcm: Vec::new(),
                    locale: "fr".into(),
                    updated: Instant::now(),
                })
                .append(bytes);
        }
        return;
    }

    if v.get("done")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let Some(buf) = sessions.get_mut(&sid) else {
            return;
        };
        let pcm = std::mem::take(&mut buf.pcm);
        buf.updated = Instant::now();
        if pcm.len() < 3200 {
            // Under 100 ms of audio: nothing worth transcribing.
            return;
        }
        let locale = buf.locale.clone();
        let (bin, model, rate) = (args.whisper_bin.clone(), args.model.clone(), args.rate);
        info!(session = %sid, bytes = pcm.len(), "transcribing utterance");
        let text = tokio::task::spawn_blocking(move || transcribe(&bin, &model, &locale, rate, &pcm))
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("join: {e}")));
        match text {
            Ok(text) if !text.is_empty() => {
                info!(session = %sid, text = %text, "transcript");
                let msg = serde_json::json!({
                    "session_id": sid,
                    "text": text,
                    "is_final": true,
                });
                if let Err(e) = client
                    .publish(response_topic, QoS::AtLeastOnce, false, msg.to_string())
                    .await
                {
                    error!(error = %e, "transcript publish failed");
                }
            }
            Ok(_) => info!(session = %sid, "utterance produced no speech"),
            Err(e) => error!(session = %sid, error = %e, "transcription failed"),
        }
        return;
    }

    // Session start message: register the buffer with its locale.
    let locale = v
        .get("locale")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("fr")
        .to_string();
    sessions.insert(
        sid,
        SessionBuf {
            pcm: Vec::new(),
            locale,
            updated: Instant::now(),
        },
    );
}

impl SessionBuf {
    fn append(&mut self, bytes: Vec<u8>) {
        self.pcm.extend_from_slice(&bytes);
        self.updated = Instant::now();
    }
}

/// Runs whisper-cli on the buffered s16le PCM and returns the cleaned text.
fn transcribe(
    bin: &PathBuf,
    model: &PathBuf,
    locale: &str,
    rate: u32,
    pcm: &[u8],
) -> anyhow::Result<String> {
    let dir = tempfile::tempdir()?;
    let wav_path = dir.path().join("utterance.wav");
    write_wav(&wav_path, pcm, rate)?;

    let lang = locale.split(['-', '_']).next().unwrap_or("fr");
    let output = Command::new(bin)
        .arg("-m")
        .arg(model)
        .args(["-l", lang, "-nt", "-np", "-f"])
        .arg(&wav_path)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "whisper-cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(clean_transcript(&String::from_utf8_lossy(&output.stdout)))
}

/// Writes s16le mono PCM into a WAV container.
fn write_wav(path: &std::path::Path, pcm: &[u8], rate: u32) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for chunk in pcm.chunks_exact(2) {
        writer.write_sample(i16::from_le_bytes([chunk[0], chunk[1]]))?;
    }
    writer.finalize()?;
    Ok(())
}

/// Normalizes whisper output for the intent matcher: collapse whitespace,
/// strip bracketed non-speech annotations ("[Musique]") and trailing
/// sentence punctuation — the matcher compares raw phrase text.
fn clean_transcript(raw: &str) -> String {
    let mut text = raw
        .split_whitespace()
        .filter(|w| !(w.starts_with('[') && w.ends_with(']')))
        .filter(|w| !(w.starts_with('(') && w.ends_with(')')))
        .collect::<Vec<_>>()
        .join(" ");
    while let Some(last) = text.chars().last() {
        if matches!(last, '.' | '!' | '?' | '…' | ',') || last.is_whitespace() {
            text.pop();
        } else {
            break;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_transcript_strips_annotations_and_punctuation() {
        assert_eq!(
            clean_transcript("  Quel heure est-il ?\n"),
            "Quel heure est-il"
        );
        assert_eq!(clean_transcript("[Musique]"), "");
        assert_eq!(
            clean_transcript(" Météo à Strasbourg ! "),
            "Météo à Strasbourg"
        );
        assert_eq!(clean_transcript("(bruit) bonjour…"), "bonjour");
    }

    #[test]
    fn wav_roundtrip_preserves_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.wav");
        let samples: Vec<i16> = vec![0, 100, -100, i16::MAX, i16::MIN];
        let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        write_wav(&path, &pcm, 16_000).unwrap();

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        let back: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(back, samples);
    }
}
