//! Athena-Voice satellite client — drives a session over MQTT.
//!
//! Opens a session on the satellite topics, injects a text utterance as a
//! final transcript (bypassing STT), prints everything the runtime sends
//! back, then closes the session:
//!
//! ```text
//! athena-voice-client --text "météo à Strasbourg"
//! ```
//!
//! Topics (see `crates/athena-voice-runtime/src/mqtt/topics.rs`):
//! - publishes `athena/sat/<sat>/session/<sid>/{start,text,end}`
//! - subscribes `athena/sat/<sat>/session/<sid>/#` for `transcript`,
//!   `tts/meta`, `tts` chunks and `done`
//! - optionally subscribes `athena/events/#` (`--events`)

use std::time::Duration;

use clap::Parser;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(about = "Text-mode satellite client for Athena-Voice")]
struct Args {
    /// MQTT broker host.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// MQTT broker port.
    #[arg(long, default_value_t = 1883)]
    port: u16,

    /// Satellite id (used as the topic segment).
    #[arg(long, default_value = "dev-sat")]
    satellite: String,

    /// Locale sent on session start.
    #[arg(long, default_value = "fr")]
    locale: String,

    /// Utterance injected as a final transcript (bypasses STT).
    #[arg(long)]
    text: String,

    /// Seconds to wait for the session to complete.
    #[arg(long, default_value_t = 15)]
    timeout_secs: u64,

    /// Also print the runtime's `athena/events/*` firehose.
    #[arg(long)]
    events: bool,

    /// Speak the answer out loud with the OS speech synthesizer (macOS
    /// `say`). Only works while the server uses the fake TTS provider,
    /// whose chunks are the answer's words rather than real audio.
    #[arg(long)]
    speak: bool,

    /// Voice passed to `say -v` when `--speak` is set.
    #[arg(long, default_value = "Thomas")]
    voice: String,

    /// Play received TTS audio chunks on the default output device.
    /// Use with a real TTS provider (e.g. the `mqtt_tts` worker), whose
    /// chunks are s16le mono PCM.
    #[arg(long)]
    play: bool,

    /// Sample rate assumed for `--play` (must match the TTS worker's).
    #[arg(long, default_value_t = 22_050)]
    rate: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let sid = Uuid::new_v4();
    let base = format!("athena/sat/{}/session/{sid}", args.satellite);

    let mut opts = MqttOptions::new(
        format!("athena-client-{}", &sid.to_string()[..8]),
        &args.host,
        args.port,
    );
    opts.set_keep_alive(Duration::from_secs(15));
    // Real TTS chunks are ~9 KiB of PCM per publish — close to rumqttc's
    // 10 KiB default cap, so give audio ample headroom.
    opts.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
    let (client, mut eventloop) = AsyncClient::new(opts, 64);

    client
        .subscribe(format!("{base}/#"), QoS::AtLeastOnce)
        .await?;
    if args.events {
        client
            .subscribe("athena/events/#", QoS::AtLeastOnce)
            .await?;
    }

    println!("session {sid} (satellite {})", args.satellite);

    let mut started = false;
    let mut end_sent = false;
    let mut tts_chunks: Vec<Vec<u8>> = Vec::new();
    let mut last_chunk_at = tokio::time::Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout_secs);
    let mut tick = tokio::time::interval(Duration::from_millis(300));

    loop {
        let ev = tokio::select! {
            ev = eventloop.poll() => ev,
            _ = tick.tick() => {
                // The runtime keeps the session open until the satellite ends
                // it (that's when `done` fires). For a one-shot text query we
                // end once the TTS stream has been quiet for a moment.
                if started
                    && !end_sent
                    && !tts_chunks.is_empty()
                    && last_chunk_at.elapsed() > Duration::from_millis(1200)
                {
                    end_sent = true;
                    client
                        .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
                        .await?;
                }
                continue;
            }
            () = tokio::time::sleep_until(deadline) => {
                eprintln!("⏰ timed out after {}s — is the server running with skills loaded?", args.timeout_secs);
                let _ = client
                    .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
                    .await;
                std::process::exit(1);
            }
        };
        match ev {
            Ok(Event::Incoming(Packet::SubAck(_))) if !started => {
                // Subscription is live — safe to open the session now.
                started = true;
                println!("→ start {{\"locale\":\"{}\"}}", args.locale);
                client
                    .publish(
                        format!("{base}/start"),
                        QoS::AtLeastOnce,
                        false,
                        serde_json::json!({ "locale": args.locale }).to_string(),
                    )
                    .await?;
                println!("→ text  {:?}", args.text);
                client
                    .publish(
                        format!("{base}/text"),
                        QoS::AtLeastOnce,
                        false,
                        args.text.clone(),
                    )
                    .await?;
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let chunks_before = tts_chunks.len();
                if handle_publish(&base, &p.topic, &p.payload, &mut tts_chunks) {
                    break;
                }
                if tts_chunks.len() != chunks_before {
                    last_chunk_at = tokio::time::Instant::now();
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("mqtt error: {e} (is the broker up at {}:{}?)", args.host, args.port);
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }

    let sentence = print_speech(&tts_chunks);

    let _ = client
        .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
        .await;
    // Give the final publish a beat to flush before dropping the event loop.
    let _ = tokio::time::timeout(Duration::from_millis(300), eventloop.poll()).await;

    if args.speak {
        match &sentence {
            Some(s) => speak(s, &args.voice),
            None => eprintln!(
                "--speak skipped: the TTS chunks were not text (real audio \
                 provider?) or no answer arrived"
            ),
        }
    }
    if args.play {
        if sentence.is_some() {
            eprintln!(
                "--play skipped: chunks are text (fake TTS provider) — use \
                 --speak instead, or run the server with a real mqtt_tts worker"
            );
        } else if !tts_chunks.is_empty() {
            play_pcm(&tts_chunks, args.rate)?;
        }
    }
    Ok(())
}

/// Plays collected s16le mono PCM chunks on the default output device.
fn play_pcm(tts_chunks: &[Vec<u8>], rate: u32) -> anyhow::Result<()> {
    use rodio::buffer::SamplesBuffer;

    let samples: Vec<i16> = tts_chunks
        .iter()
        .flat_map(|c| c.chunks_exact(2))
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    println!(
        "▶️  playing {:.1}s of audio…",
        samples.len() as f32 / rate as f32
    );
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = rodio::Sink::try_new(&handle)?;
    sink.append(SamplesBuffer::new(1, rate, samples));
    sink.sleep_until_end();
    Ok(())
}

/// Reads the answer aloud via the OS speech synthesizer.
fn speak(sentence: &str, voice: &str) {
    if !cfg!(target_os = "macos") {
        eprintln!("--speak currently uses macOS `say`; skipping on this OS");
        return;
    }
    let with_voice = std::process::Command::new("say")
        .args(["-v", voice])
        .arg(sentence)
        .status();
    match with_voice {
        Ok(status) if status.success() => {}
        // Unknown voice (or other failure): retry with the system default.
        _ => {
            let _ = std::process::Command::new("say").arg(sentence).status();
        }
    }
}

/// Prints one incoming message; returns `true` when the session is done.
fn handle_publish(base: &str, topic: &str, payload: &[u8], tts_chunks: &mut Vec<Vec<u8>>) -> bool {
    if let Some(kind) = topic.strip_prefix(base).and_then(|s| s.strip_prefix('/')) {
        match kind {
            // Our own publishes, echoed back by the wildcard subscription.
            "start" | "text" | "end" => {}
            "transcript" => println!("📝 {}", String::from_utf8_lossy(payload)),
            "tts/meta" => println!("🎧 {}", String::from_utf8_lossy(payload)),
            "tts" => tts_chunks.push(payload.to_vec()),
            "done" => {
                println!("✅ {}", String::from_utf8_lossy(payload));
                return true;
            }
            other => println!("? {other}: {}", String::from_utf8_lossy(payload)),
        }
    } else if let Some(kind) = topic.strip_prefix("athena/events/") {
        println!("⚡ {kind}: {}", String::from_utf8_lossy(payload));
    }
    false
}

/// With the fake TTS provider each chunk is one word of the spoken answer,
/// so valid-UTF-8 chunks are printed (and returned) as the sentence; real
/// audio falls back to a byte count.
fn print_speech(tts_chunks: &[Vec<u8>]) -> Option<String> {
    if tts_chunks.is_empty() {
        return None;
    }
    let total: usize = tts_chunks.iter().map(Vec::len).sum();
    match tts_chunks
        .iter()
        .map(|c| std::str::from_utf8(c))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(words) => {
            let sentence = words.join(" ");
            println!("🔊 {sentence}");
            Some(sentence)
        }
        Err(_) => {
            println!("🔊 {} audio chunks ({total} bytes)", tts_chunks.len());
            None
        }
    }
}
