mod config;
mod mic;
mod net;
mod session;
mod speaker;

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
        Frame(Vec<u8>),
        UtteranceEnd,
        Mqtt(String, Vec<u8>),
        Tick,
    }

    pub fn run() {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();
        info!("athena-satellite-esp32 boot");

        let cfg = &config::CONFIG;
        let peripherals = Peripherals::take().expect("peripherals");
        let sysloop = EspSystemEventLoop::take().expect("sysloop");

        let _wifi = net::connect_wifi(peripherals.modem, sysloop, cfg.wifi_ssid, cfg.wifi_pass)
            .expect("wifi");

        // Unbounded on purpose: a stalled broker surfaces as memory
        // pressure, bounded by the 10 s utterance cap in session.rs.
        let (tx, rx) = channel::<AppEvent>();

        // MQTT inbound → events.
        let (mqtt_tx, mqtt_rx) = channel::<(String, Vec<u8>)>();
        let mut mqtt = net::Mqtt::connect(cfg.mqtt_url, cfg.sat_id, mqtt_tx).expect("mqtt");
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
            let mut button =
                PinDriver::input(peripherals.pins.gpio0).expect("button pin");
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
        // utterance via the silence tracker.
        let streaming = Arc::new(AtomicBool::new(false));
        spawn("mic", {
            let tx = tx.clone();
            let streaming = Arc::clone(&streaming);
            let mut m = mic::driver::Mic::new(
                peripherals.i2s0,
                peripherals.pins.gpio4,
                peripherals.pins.gpio5,
                peripherals.pins.gpio6,
            )
            .expect("mic i2s");
            move || {
                let mut tracker =
                    mic::SilenceTracker::new(mic::SILENCE_RMS, mic::SILENCE_MS, 20);
                let mut frame = Vec::with_capacity(mic::FRAME_SAMPLES * 2);
                let mut was_streaming = false;
                loop {
                    if !streaming.load(Ordering::Relaxed) {
                        was_streaming = false;
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    if !was_streaming {
                        tracker.reset();
                        was_streaming = true;
                    }
                    match m.read_frame(&mut frame) {
                        Ok(samples) => {
                            let done = tracker.push(samples);
                            let _ = tx.send(AppEvent::Frame(frame.clone()));
                            if done {
                                streaming.store(false, Ordering::Relaxed);
                                let _ = tx.send(AppEvent::UtteranceEnd);
                            }
                        }
                        Err(e) => {
                            error!("mic read failed: {e}");
                            std::thread::sleep(Duration::from_millis(100));
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

        let spk = speaker::Speaker::new(
            peripherals.i2s1,
            peripherals.pins.gpio15,
            peripherals.pins.gpio16,
            peripherals.pins.gpio7,
        )
        .expect("speaker i2s");

        info!("ready (sat_id={}): press BOOT to talk", cfg.sat_id);

        let boot = Instant::now();
        let mut session = Session::new(cfg.sat_id, cfg.locale);
        while let Ok(event) = rx.recv() {
            let input = match event {
                AppEvent::Button => Input::Trigger,
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
                    Command::SessionEnded => {
                        streaming.store(false, Ordering::Relaxed);
                        info!("session ended, idle");
                    }
                }
            }
            if *session.state() == State::Streaming {
                streaming.store(true, Ordering::Relaxed);
            }
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
