//! Athena-Voice TTS worker — bridges the generic MQTT TTS protocol to a
//! local speech synthesizer.
//!
//! Wire protocol (see `athena-voice-providers/src/remote/mqtt_tts.rs`):
//! - requests arrive on `athena/providers/tts/<name>/request` as JSON
//!   `{ "session_id": "<uuid>", "locale": "fr", "text": "..." }`
//! - responses go to `athena/providers/tts/<name>/response` as JSON
//!   `{ "session_id", "chunk_b64", "done" }` — base64-encoded audio
//!   chunks, terminated by a `done: true` marker.
//! - the FIRST response message also carries the real audio format:
//!   `{ "format": "s16le", "sample_rate": <hz>, "channels": 1 }`. The
//!   runtime forwards this to satellites as `tts/meta`, so it must
//!   describe the actual bytes. Older workers omitting the fields are
//!   read as s16le/22050.
//!
//! Two engines, same wire protocol: `--engine say` (macOS `say` +
//! `afconvert`, the default) and `--engine piper --piper-model
//! <voice.onnx>` (portable, Linux included). Piper speaks at its
//! model's native rate, which is read from its WAV output and declared
//! in the metadata — nothing resamples.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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

    /// Synthesis engine. `say` is macOS-only; `piper` is portable
    /// (Linux included) and needs --piper-model.
    #[arg(long, value_enum, default_value_t = Engine::Say)]
    engine: Engine,

    /// Voice passed to `say -v` for French locales.
    #[arg(long, default_value = "Thomas")]
    voice: String,

    /// `piper` executable (or `python3 -m piper` wrapper) on PATH.
    #[arg(long, default_value = "piper")]
    piper_bin: String,

    /// Path to the Piper voice model (`.onnx`). Its companion
    /// `.onnx.json` must sit next to it. Not vendored — see "Voice on
    /// Linux (Piper)" in the README for where to fetch one.
    #[arg(long)]
    piper_model: Option<PathBuf>,

    /// Output sample rate in Hz (s16le mono). Ignored by `piper`, which
    /// speaks at its model's native rate (declared in the metadata).
    #[arg(long, default_value_t = 22_050)]
    rate: u32,

    /// Chunk size in milliseconds.
    #[arg(long, default_value_t = 200)]
    chunk_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Engine {
    /// macOS `say` + `afconvert`.
    Say,
    /// Piper CLI: `--model <onnx> --output_file <wav>`, text on stdin.
    Piper,
}

/// Validated engine settings, cloned into each request task.
#[derive(Debug, Clone)]
enum EngineConfig {
    Say { voice: String, rate: u32 },
    Piper { bin: String, model: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    // Fail fast, before any MQTT traffic: a worker that can't
    // synthesize is worse than an absent one (the runtime then speaks a
    // locale-aware apology instead of hanging).
    let engine_cfg = match args.engine {
        Engine::Say => {
            anyhow::ensure!(
                Command::new("say").arg("--version").output().is_ok(),
                "`say` not found — the `say` engine requires macOS; use \
                 --engine piper --piper-model <voice.onnx> elsewhere"
            );
            EngineConfig::Say {
                voice: args.voice.clone(),
                rate: args.rate,
            }
        }
        Engine::Piper => {
            let model = args.piper_model.clone().ok_or_else(|| {
                anyhow::anyhow!("--engine piper requires --piper-model <voice.onnx>")
            })?;
            anyhow::ensure!(
                model.is_file(),
                "piper model not found: {} — download a voice (see \
                 \"Voice on Linux (Piper)\" in the README)",
                model.display()
            );
            let config = model.with_extension("onnx.json");
            anyhow::ensure!(
                config.is_file(),
                "piper model config not found: {} — it ships alongside \
                 the .onnx voice and must sit next to it",
                config.display()
            );
            anyhow::ensure!(
                Command::new(&args.piper_bin)
                    .arg("--help")
                    .output()
                    .is_ok(),
                "piper binary not runnable: {} — install piper (pip \
                 install piper-tts) or pass --piper-bin <path>",
                args.piper_bin
            );
            let cfg = EngineConfig::Piper {
                bin: args.piper_bin.clone(),
                model,
            };
            // Synthesize one word before announcing readiness. `--help`
            // succeeding proves nothing about the native stack: broken
            // wheels (espeak-ng data compiled to an absent path),
            // unreadable voices and ABI mismatches all surface only on
            // real synthesis — and a worker that accepts requests it
            // cannot serve leaves every session hanging.
            let (samples, rate) = synthesize(&cfg, "test", "fr")
                .map_err(|e| anyhow::anyhow!("piper smoke synthesis failed: {e}"))?;
            anyhow::ensure!(
                !samples.is_empty(),
                "piper produced no audio for a smoke test — check the \
                 voice model and its .onnx.json"
            );
            info!(rate, samples = samples.len(), "piper ready");
            cfg
        }
    };

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

    info!(topic = %request_topic, engine = ?args.engine, "TTS worker ready");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == request_topic => {
                let client = client.clone();
                let response_topic = response_topic.clone();
                let engine_cfg = engine_cfg.clone();
                let chunk_ms = args.chunk_ms;
                let payload = p.payload.to_vec();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle_request(&client, &response_topic, &payload, &engine_cfg, chunk_ms)
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
    engine: &EngineConfig,
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

    // Piper speaks at its model's native rate, so the rate is an
    // output of synthesis, not an input — it goes straight into the
    // declared metadata, and nothing resamples.
    let (samples, rate) = {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || synthesize(&engine, &text, &locale)).await??
    };

    // Stream fixed-duration chunks, then the done marker. The FIRST
    // message declares the real audio format; the runtime forwards it
    // to satellites as `tts/meta`, so it must not lie.
    let samples_per_chunk = (rate * chunk_ms / 1000).max(1) as usize;
    let mut declared = false;
    for chunk in samples.chunks(samples_per_chunk) {
        let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut msg = serde_json::json!({
            "session_id": session_id,
            "chunk_b64": STANDARD.encode(&bytes),
            "done": false,
        });
        if !declared {
            msg["format"] = serde_json::json!("s16le");
            msg["sample_rate"] = serde_json::json!(rate);
            msg["channels"] = serde_json::json!(1);
            declared = true;
        }
        client
            .publish(response_topic, QoS::AtLeastOnce, false, msg.to_string())
            .await?;
    }
    // Empty text yields no chunks; the done marker then carries the
    // format so the runtime still publishes honest metadata.
    let done = if declared {
        serde_json::json!({ "session_id": session_id, "done": true })
    } else {
        serde_json::json!({
            "session_id": session_id,
            "done": true,
            "format": "s16le",
            "sample_rate": rate,
            "channels": 1,
        })
    };
    client
        .publish(response_topic, QoS::AtLeastOnce, false, done.to_string())
        .await?;
    Ok(())
}

/// Synthesizes `text` to s16le mono samples, returning them with the
/// rate they are actually at.
fn synthesize(
    engine: &EngineConfig,
    text: &str,
    locale: &str,
) -> anyhow::Result<(Vec<i16>, u32)> {
    match engine {
        EngineConfig::Say { voice, rate } => {
            Ok((synthesize_say(text, locale, voice, *rate)?, *rate))
        }
        EngineConfig::Piper { bin, model } => synthesize_piper(bin, model, text),
    }
}

/// Piper CLI: `--model <onnx> --output_file <wav>`, text on stdin (the
/// form both piper1-gpl and the legacy binary accept). The WAV header
/// carries the model's native rate, which we return rather than assume.
fn synthesize_piper(
    bin: &str,
    model: &std::path::Path,
    text: &str,
) -> anyhow::Result<(Vec<i16>, u32)> {
    let dir = tempfile::tempdir()?;
    let wav = dir.path().join("out.wav");

    let mut child = Command::new(bin)
        .arg("--model")
        .arg(model)
        .arg("--output_file")
        .arg(&wav)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("piper stdin unavailable"))?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    anyhow::ensure!(status.success(), "piper failed ({status})");

    let mut reader = hound::WavReader::open(&wav)?;
    let spec = reader.spec();
    let samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
    // Piper voices are mono; downmix defensively rather than emitting
    // interleaved stereo as if it were mono.
    let samples = if spec.channels > 1 {
        samples
            .chunks(spec.channels as usize)
            .map(|f| (f.iter().map(|s| i32::from(*s)).sum::<i32>() / i32::from(spec.channels)) as i16)
            .collect()
    } else {
        samples
    };
    Ok((samples, spec.sample_rate))
}

/// Synthesizes `text` to s16le mono samples at `rate` via `say` + `afconvert`.
fn synthesize_say(text: &str, locale: &str, voice: &str, rate: u32) -> anyhow::Result<Vec<i16>> {
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
    use super::*;

    /// Writes a WAV `stub_piper` will hand back, so tests exercise the
    /// real invocation without installing Piper.
    fn write_wav(path: &std::path::Path, rate: u32, channels: u16, frames: usize) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            for c in 0..channels {
                // Distinct per-channel values so a bad downmix shows up.
                w.write_sample(((i as i16) + 1) * (1 + c as i16) * 100)
                    .unwrap();
            }
        }
        w.finalize().unwrap();
    }

    /// A `piper` stand-in: consumes stdin, honours `--output_file`.
    fn stub_piper(dir: &std::path::Path, canned_wav: &std::path::Path) -> String {
        let script = dir.join("piper-stub.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat > /dev/null\nwhile [ $# -gt 0 ]; do \
                 if [ \"$1\" = \"--output_file\" ]; then out=\"$2\"; fi; \
                 shift; done\ncp {} \"$out\"\n",
                canned_wav.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        script.to_string_lossy().into_owned()
    }

    #[test]
    fn piper_reports_the_models_native_rate_not_a_guess() {
        let dir = tempfile::tempdir().unwrap();
        let canned = dir.path().join("canned.wav");
        // A rate that is neither the worker default (22050) nor the old
        // hardcoded meta value (24000).
        write_wav(&canned, 16_000, 1, 4);
        let bin = stub_piper(dir.path(), &canned);

        let (samples, rate) =
            synthesize_piper(&bin, &dir.path().join("voice.onnx"), "bonjour").unwrap();
        assert_eq!(rate, 16_000, "the WAV header's rate must be reported");
        assert_eq!(samples, vec![100, 200, 300, 400]);
    }

    #[test]
    fn piper_downmixes_multichannel_output_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let canned = dir.path().join("canned.wav");
        write_wav(&canned, 22_050, 2, 2);
        let bin = stub_piper(dir.path(), &canned);

        let (samples, rate) =
            synthesize_piper(&bin, &dir.path().join("voice.onnx"), "bonjour").unwrap();
        assert_eq!(rate, 22_050);
        // Frames are (100,200) and (200,400) → means 150 and 300.
        assert_eq!(samples, vec![150, 300]);
    }

    #[test]
    fn piper_failure_is_an_error_not_silent_empty_audio() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fail.sh");
        std::fs::write(&script, "#!/bin/sh\ncat > /dev/null\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let err = synthesize_piper(
            &script.to_string_lossy(),
            &dir.path().join("voice.onnx"),
            "bonjour",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("piper failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn chunking_math_never_zero() {
        // rate * chunk_ms / 1000 could truncate to 0 for tiny values; the
        // worker clamps to 1 so `chunks()` can't panic.
        let samples_per_chunk = (8u32 * 10 / 1000).max(1) as usize;
        assert_eq!(samples_per_chunk, 1);
    }
}
