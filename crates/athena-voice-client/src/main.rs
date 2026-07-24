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
    #[arg(long, conflicts_with_all = ["wav", "microphone"])]
    text: Option<String>,

    /// WAV file to send as session audio (any rate/format; converted to
    /// s16le mono 16 kHz — the STT pipeline contract).
    #[arg(long, conflicts_with = "microphone")]
    wav: Option<std::path::PathBuf>,

    /// Record from the default input device and send it as session audio.
    #[arg(long)]
    microphone: bool,

    /// Recording duration for --microphone, in seconds.
    #[arg(long, default_value_t = 5)]
    duration_secs: u64,

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

    /// Print a per-stage latency breakdown after the session completes.
    #[arg(long)]
    timing: bool,
}

/// Per-stage timestamps, taken when the corresponding message arrives back
/// from the broker (so every figure includes real MQTT round-trips).
#[derive(Default)]
struct Timing {
    /// Our own input echoed back: text message, or the empty end-of-utterance
    /// audio marker.
    input_done: Option<std::time::Instant>,
    /// First transcript from the runtime (STT output; echo-of-text for --text).
    transcript: Option<std::time::Instant>,
    /// First tts/text — the answer exists as text (intent + skill done).
    answer_text: Option<std::time::Instant>,
    first_audio: Option<std::time::Instant>,
    last_audio: Option<std::time::Instant>,
}

impl Timing {
    fn mark(slot: &mut Option<std::time::Instant>) {
        if slot.is_none() {
            *slot = Some(std::time::Instant::now());
        }
    }

    fn print(&self, voice_input: bool) {
        fn span(a: Option<std::time::Instant>, b: Option<std::time::Instant>) -> String {
            match (a, b) {
                (Some(a), Some(b)) if b >= a => format!("{:>6} ms", (b - a).as_millis()),
                _ => "     — ".to_string(),
            }
        }
        println!("⏱  timing:");
        if voice_input {
            println!(
                "    audio end → transcript   : {}   (STT)",
                span(self.input_done, self.transcript)
            );
        }
        println!(
            "    transcript → answer text : {}   (intent + skill)",
            span(self.transcript, self.answer_text)
        );
        println!(
            "    answer text → first audio: {}   (TTS synthesis)",
            span(self.answer_text, self.first_audio)
        );
        println!(
            "    first → last audio chunk : {}   (streaming)",
            span(self.first_audio, self.last_audio)
        );
        println!(
            "    input end → last audio   : {}   (TOTAL response)",
            span(self.input_done, self.last_audio)
        );
    }
}

/// STT pipeline contract: s16le mono at 16 kHz.
const STT_RATE: u32 = 16_000;
/// 100 ms of s16le audio at the STT rate.
const AUDIO_CHUNK_BYTES: usize = (STT_RATE as usize / 10) * 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let sid = Uuid::new_v4();
    let base = format!("athena/sat/{}/session/{sid}", args.satellite);

    // Resolve the utterance source up front: audio is fully captured/decoded
    // before the session opens, which keeps the session logic identical for
    // all three modes.
    let audio: Option<Vec<u8>> = if let Some(path) = &args.wav {
        Some(read_wav_as_s16le_16k(path)?)
    } else if args.microphone {
        Some(record_microphone(args.duration_secs)?)
    } else if args.text.is_none() {
        anyhow::bail!("one of --text, --wav, or --microphone is required");
    } else {
        None
    };

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
    let mut timing = Timing::default();
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
                // The publish above only ENQUEUES — without polling the
                // event loop it never reaches the broker and the session
                // leaks server-side until shutdown.
                flush_eventloop(&mut eventloop).await;
                // process::exit skips buffered-stdout flushing (lines are
                // block-buffered when piped) — flush or lose the session log.
                let _ = std::io::Write::flush(&mut std::io::stdout());
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
                if let Some(text) = &args.text {
                    println!("→ text  {text:?}");
                    client
                        .publish(format!("{base}/text"), QoS::AtLeastOnce, false, text.clone())
                        .await?;
                } else if let Some(pcm) = audio.clone() {
                    #[allow(clippy::cast_precision_loss)]
                    let secs = pcm.len() as f32 / (STT_RATE as f32 * 2.0);
                    println!("→ audio {secs:.1}s ({} bytes)", pcm.len());
                    // Feed from a task: publishing enqueues on a bounded
                    // channel drained by this event loop, so long audio
                    // would deadlock if sent inline here.
                    let audio_client = client.clone();
                    let audio_topic = format!("{base}/audio");
                    tokio::spawn(async move {
                        for chunk in pcm.chunks(AUDIO_CHUNK_BYTES) {
                            if audio_client
                                .publish(&audio_topic, QoS::AtLeastOnce, false, chunk.to_vec())
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        // Empty frame = end-of-utterance marker: tells the
                        // STT provider to flush a transcript now.
                        let _ = audio_client
                            .publish(&audio_topic, QoS::AtLeastOnce, false, "")
                            .await;
                    });
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let chunks_before = tts_chunks.len();
                if handle_publish(&base, &p.topic, &p.payload, &mut tts_chunks, &mut timing) {
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
    if args.timing {
        // In --text mode there is no STT stage; the injected text's echo
        // doubles as the transcript timestamp.
        if timing.transcript.is_none() {
            timing.transcript = timing.input_done;
        }
        timing.print(audio.is_some());
    }

    let _ = client
        .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
        .await;
    flush_eventloop(&mut eventloop).await;

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

/// Polls the event loop until queued publishes (the session `end`) are
/// acknowledged or ~500 ms passes — publishing only enqueues; without this
/// the final message never leaves the process.
async fn flush_eventloop(eventloop: &mut rumqttc::EventLoop) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), eventloop.poll()).await {
            Ok(Ok(Event::Incoming(Packet::PubAck(_)))) => return,
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

/// Reads any WAV file and converts it to the STT contract: s16le mono 16 kHz.
fn read_wav_as_s16le_16k(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| anyhow::anyhow!("cannot open {}: {e}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = f32::from(i16::MAX);
            reader
                .samples::<i16>()
                .map(|s| s.map(|v| f32::from(v) / max))
                .collect::<Result<_, _>>()?
        }
    };
    Ok(to_s16le_16k(&samples, spec.sample_rate, spec.channels))
}

/// Records `secs` seconds from the default input device, converted to the
/// STT contract format.
fn record_microphone(secs: u64) -> anyhow::Result<Vec<u8>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input device (microphone)"))?;
    let config = device.default_input_config()?;
    let (rate, channels) = (config.sample_rate().0, config.channels());

    println!(
        "🎙  recording {secs}s from {:?} — speak now…",
        device.name().unwrap_or_else(|_| "input".into())
    );
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
    let cb_buf = buf.clone();
    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _| cb_buf.lock().unwrap().extend_from_slice(data),
        |e| eprintln!("input stream error: {e}"),
        None,
    )?;
    stream.play()?;
    std::thread::sleep(std::time::Duration::from_secs(secs));
    drop(stream);

    let samples = std::mem::take(&mut *buf.lock().unwrap());
    anyhow::ensure!(!samples.is_empty(), "microphone produced no samples");
    Ok(to_s16le_16k(&samples, rate, channels))
}

/// Downmixes interleaved f32 samples to mono and linearly resamples to
/// 16 kHz s16le bytes.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_s16le_16k(samples: &[f32], src_rate: u32, channels: u16) -> Vec<u8> {
    let channels = channels.max(1) as usize;
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    if mono.is_empty() {
        return Vec::new();
    }
    let ratio = f64::from(src_rate) / f64::from(STT_RATE);
    let out_len = (mono.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len * 2);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono[idx.min(mono.len() - 1)];
        let b = mono[(idx + 1).min(mono.len() - 1)];
        let v = (a + (b - a) * frac).clamp(-1.0, 1.0);
        out.extend_from_slice(&((v * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    out
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
fn handle_publish(
    base: &str,
    topic: &str,
    payload: &[u8],
    tts_chunks: &mut Vec<Vec<u8>>,
    timing: &mut Timing,
) -> bool {
    if let Some(kind) = topic.strip_prefix(base).and_then(|s| s.strip_prefix('/')) {
        match kind {
            // Our own publishes, echoed back by the wildcard subscription.
            // The echoed text / end-of-utterance marker stamps "input done".
            "text" => Timing::mark(&mut timing.input_done),
            "audio" if payload.is_empty() => Timing::mark(&mut timing.input_done),
            "start" | "audio" | "end" => {}
            "transcript" => {
                Timing::mark(&mut timing.transcript);
                println!("📝 {}", String::from_utf8_lossy(payload));
            }
            "tts/meta" => println!("🎧 {}", String::from_utf8_lossy(payload)),
            "tts/text" => {
                // The answer as text, published by the runtime alongside the
                // synthesized audio.
                let text = serde_json::from_slice::<serde_json::Value>(payload)
                    .ok()
                    .and_then(|v| {
                        v.get("text")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| String::from_utf8_lossy(payload).into_owned());
                println!("🗣  {text}");
                Timing::mark(&mut timing.answer_text);
            }
            "tts" => {
                Timing::mark(&mut timing.first_audio);
                timing.last_audio = Some(std::time::Instant::now());
                tts_chunks.push(payload.to_vec());
            }
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
