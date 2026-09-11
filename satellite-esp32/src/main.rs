mod config;
mod mic;
mod net;
mod session;
mod speaker;
mod wake;

#[cfg(feature = "hardware")]
fn main() {
    hw::run();
}

#[cfg(not(feature = "hardware"))]
fn main() {}

#[cfg(feature = "hardware")]
mod hw {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    use esp_idf_svc::eventloop::EspSystemEventLoop;
    use esp_idf_svc::hal::gpio::PinDriver;
    use esp_idf_svc::hal::peripherals::Peripherals;
    use log::{error, info};

    use crate::session::{Command, Input, Session, State};
    use crate::{config, mic, net, speaker};

    enum AppEvent {
        Button,
        /// Wake word detected — same effect as Button. Only the S3 mic
        /// path constructs it (WakeNet models are S3-only).
        #[cfg_attr(not(esp32s3), allow(dead_code))]
        Wake,
        Frame(Vec<u8>),
        UtteranceEnd,
        Mqtt(String, Vec<u8>),
        Tick,
    }

    /// Per-chip wiring (see config.rs / README). The chip cfgs come from
    /// esp-idf-sys via build.rs.
    struct Wiring {
        mic_bclk: esp_idf_svc::hal::gpio::AnyIOPin,
        mic_ws: esp_idf_svc::hal::gpio::AnyIOPin,
        mic_sd: esp_idf_svc::hal::gpio::AnyIOPin,
        spk_bclk: esp_idf_svc::hal::gpio::AnyIOPin,
        spk_lrc: esp_idf_svc::hal::gpio::AnyIOPin,
        spk_din: esp_idf_svc::hal::gpio::AnyIOPin,
        /// BOOT button (GPIO0 on both chips): push-to-talk.
        button: esp_idf_svc::hal::gpio::Gpio0,
    }

    /// Classic ESP32 (WROOM): GPIO6–11 belong to the internal flash, so
    /// the mic sits on 32/25/33 and the amp on 27/26/22.
    #[cfg(esp32)]
    fn wiring(pins: esp_idf_svc::hal::gpio::Pins) -> Wiring {
        use esp_idf_svc::hal::gpio::IOPin;
        Wiring {
            mic_bclk: pins.gpio32.downgrade(),
            mic_ws: pins.gpio25.downgrade(),
            mic_sd: pins.gpio33.downgrade(),
            spk_bclk: pins.gpio27.downgrade(),
            spk_lrc: pins.gpio26.downgrade(),
            spk_din: pins.gpio22.downgrade(),
            button: pins.gpio0,
        }
    }

    /// ESP32-S3. Pins chosen to exist on the XIAO ESP32S3 header too
    /// (mic = D3/D4/D5, amp = D8/D9/D10) — GPIO15/16 are camera pins on
    /// the XIAO Sense and not exposed at all.
    #[cfg(esp32s3)]
    fn wiring(pins: esp_idf_svc::hal::gpio::Pins) -> Wiring {
        use esp_idf_svc::hal::gpio::IOPin;
        Wiring {
            mic_bclk: pins.gpio4.downgrade(),
            mic_ws: pins.gpio5.downgrade(),
            mic_sd: pins.gpio6.downgrade(),
            spk_bclk: pins.gpio7.downgrade(),
            spk_lrc: pins.gpio8.downgrade(),
            spk_din: pins.gpio9.downgrade(),
            button: pins.gpio0,
        }
    }

    pub fn run() {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();
        info!("athena-satellite-esp32 boot");

        let cfg = &config::CONFIG;
        config::log_lengths();
        let topic_root = config::checked(cfg.topic_root, "topic_root", "athena");
        let sat_id = config::checked(cfg.sat_id, "sat_id", "esp32-sat");
        let locale = config::checked(cfg.locale, "locale", "fr");

        // Boot self-test, before any peripheral or radio is touched: the
        // first heap-allocating `format!` and the topic builder that a
        // device reported panicking on ("capacity overflow" inside
        // format!). Running it here separates a broken heap/flash
        // mapping — which fails immediately — from corruption introduced
        // later by Wi-Fi, PSRAM or the I2S drivers.
        let probe = format!("{topic_root}-{sat_id}");
        info!("self-test: format! ok (len {})", probe.len());
        let mqtt_url = config::checked(cfg.mqtt_url, "mqtt_url", "mqtt://127.0.0.1:1883");
        // Everything MQTT needs is built here, while the config is known
        // good, rather than re-read after Wi-Fi bring-up.
        let filters = Session::subscriptions(topic_root, sat_id);
        let client_id = format!("athena-sat-{sat_id}");
        info!("self-test: subscriptions ok ({})", filters[0]);

        // These lengths are read (never the contents) after each
        // initialisation step. A jump to an implausible value names the
        // statement that corrupts these locals — the device fails in
        // Session::subscriptions after Wi-Fi with inputs that were sound
        // moments earlier.
        let probe_lens = |stage: &str, root: &str, sat: &str| {
            info!("probe {stage}: root_len={} sat_len={}", root.len(), sat.len());
        };
        probe_lens("0-start", topic_root, sat_id);

        let peripherals = Peripherals::take().expect("peripherals");
        let sysloop = EspSystemEventLoop::take().expect("sysloop");
        probe_lens("1-peripherals", topic_root, sat_id);
        let w = wiring(peripherals.pins);
        probe_lens("2-wiring", topic_root, sat_id);

        let _wifi = net::connect_wifi(peripherals.modem, sysloop, cfg.wifi_ssid, cfg.wifi_pass)
            .expect("wifi");
        probe_lens("3-wifi", topic_root, sat_id);

        // Unbounded on purpose: a stalled broker surfaces as memory
        // pressure, bounded by the 10 s utterance cap in session.rs.
        let (tx, rx) = channel::<AppEvent>();

        // MQTT inbound → events.
        let (mqtt_tx, mqtt_rx) = channel::<(String, Vec<u8>)>();
        probe_lens("4-channels", topic_root, sat_id);
        let mut mqtt = net::Mqtt::connect(net::Setup {
            url: mqtt_url,
            client_id: &client_id,
            filters: &filters,
            tx: mqtt_tx,
        })
        .expect("mqtt");
        spawn("mqtt-fwd", {
            let tx = tx.clone();
            move || {
                while let Ok((topic, payload)) = mqtt_rx.recv() {
                    let _ = tx.send(AppEvent::Mqtt(topic, payload));
                }
            }
        });

        // BOOT button (active low, pull-up), polled with debounce.
        spawn("button", {
            let tx = tx.clone();
            let mut button = PinDriver::input(w.button).expect("button pin");
            button
                .set_pull(esp_idf_svc::hal::gpio::Pull::Up)
                .expect("button pull-up");
            move || {
                let mut was_low = false;
                loop {
                    let low = button.is_low();
                    if low && !was_low {
                        let _ = tx.send(AppEvent::Button);
                    }
                    was_low = low;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });

        // Mic: streams frames while the flag is set; signals end of
        // utterance via the silence tracker. When idle+armed on an S3,
        // frames feed WakeNet instead ("Alexa" → same as a BOOT press).
        let streaming = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(true));
        spawn("mic", {
            let tx = tx.clone();
            let streaming = Arc::clone(&streaming);
            let armed = Arc::clone(&armed);
            let mut m = mic::driver::Mic::new(peripherals.i2s0, w.mic_bclk, w.mic_ws, w.mic_sd)
                .expect("mic i2s");
            move || {
                let mut tracker =
                    mic::SilenceTracker::new(mic::SILENCE_RMS, mic::SILENCE_MS, 20);
                let mut frame = Vec::with_capacity(mic::FRAME_SAMPLES * 2);
                let mut was_streaming = false;
                let mut spoke = false;
                let mut frames_sent: u32 = 0;
                // Raw input level once per second (50 × 20 ms frames) —
                // the monitor's proof that the mic is wired right:
                // silence sits well under 100, speech in the thousands.
                let mut meter = mic::LevelMeter::new(50);
                #[cfg(esp32s3)]
                let mut wakenet = crate::wake::WakeNet::new();
                #[cfg(not(esp32s3))]
                let _ = &armed; // classic ESP32: push-to-talk only
                loop {
                    let samples = match m.read_frame(&mut frame) {
                        Ok(samples) => samples,
                        Err(e) => {
                            error!("mic read failed: {e}");
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    };
                    if let Some((rms, peak)) = meter.push(samples) {
                        info!("mic: rms {rms:>5} peak {peak:>5}");
                    }
                    if streaming.load(Ordering::Relaxed) {
                        if !was_streaming {
                            tracker.reset();
                            was_streaming = true;
                            spoke = false;
                            frames_sent = 0;
                        }
                        let done = tracker.push(samples);
                        if !spoke && tracker.speech_started() {
                            spoke = true;
                            info!("speech started");
                        }
                        let _ = tx.send(AppEvent::Frame(frame.clone()));
                        frames_sent += 1;
                        if done {
                            info!("end of utterance ({} ms sent)", frames_sent * 20);
                            streaming.store(false, Ordering::Relaxed);
                            let _ = tx.send(AppEvent::UtteranceEnd);
                        }
                        continue;
                    }
                    was_streaming = false;
                    // Idle frames teach the tracker the room's noise
                    // floor, so the silence threshold fits any mic/room.
                    tracker.observe(samples);
                    #[cfg(esp32s3)]
                    if armed.load(Ordering::Relaxed) {
                        if let Some(wn) = wakenet.as_mut() {
                            if wn.feed(samples) {
                                let _ = tx.send(AppEvent::Wake);
                            }
                        }
                    }
                }
            }
        });

        // Timeout ticker.
        spawn("ticker", {
            let tx = tx.clone();
            move || {
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    let _ = tx.send(AppEvent::Tick);
                }
            }
        });

        let spk = speaker::Speaker::new(peripherals.i2s1, w.spk_bclk, w.spk_lrc, w.spk_din)
            .expect("speaker i2s");

        info!(
            "ready (root={topic_root}, sat_id={sat_id}): press BOOT to talk"
        );

        let boot = Instant::now();
        let mut session = Session::new(topic_root, sat_id, locale);
        while let Ok(event) = rx.recv() {
            let input = match event {
                AppEvent::Button => Input::Trigger,
                AppEvent::Wake => {
                    info!("wake word detected");
                    Input::Trigger
                }
                AppEvent::Frame(bytes) => Input::MicFrame(bytes),
                AppEvent::UtteranceEnd => Input::SilenceDetected,
                AppEvent::Mqtt(topic, payload) => Input::Inbound { topic, payload },
                AppEvent::Tick => Input::Tick,
            };
            let now_ms = boot.elapsed().as_millis() as u64;
            for command in session.handle(input, now_ms) {
                match command {
                    Command::Publish { topic, payload } => {
                        if let Err(e) = mqtt.publish(&topic, &payload) {
                            error!("publish to {topic} failed: {e}");
                        }
                    }
                    Command::ConfigureSpeaker { sample_rate } => {
                        spk.set_sample_rate(sample_rate);
                    }
                    Command::Play(bytes) => spk.enqueue(bytes),
                    Command::ShowTranscript { text, is_final } => {
                        info!("heard{}: {text}", if is_final { "" } else { " (partial)" });
                    }
                    Command::ShowAnswer(text) => info!("answer: {text}"),
                    Command::SessionEnded => {
                        streaming.store(false, Ordering::Relaxed);
                        if cfg!(esp32s3) {
                            info!("session ended — listening for the wake word (or BOOT)");
                        } else {
                            info!("session ended — press BOOT to talk");
                        }
                    }
                }
            }
            if *session.state() == State::Streaming {
                streaming.store(true, Ordering::Relaxed);
            }
            // Wake detection only listens while the session is idle, so
            // TTS playback can't retrigger it.
            armed.store(*session.state() == State::Idle, Ordering::Relaxed);
        }
    }

    fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .name(name.into())
            .stack_size(6 * 1024)
            .spawn(f)
            .expect("spawn thread");
    }

}
