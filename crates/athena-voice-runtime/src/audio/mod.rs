//! Audio playback sink for skill-driven chunks.
//!
//! Behind the `audio` feature: consumes [`Event::AudioChunk`] and
//! [`Event::VolumeChanged`] from the event bus and plays samples on the
//! default output device via `rodio` (cross-platform — CoreAudio on macOS,
//! ALSA/PulseAudio on Linux).
//!
//! `rodio`'s `OutputStream` is `!Send`, so the device lives on a dedicated
//! OS thread fed through a std channel; the async side only decodes chunk
//! payloads into `f32` samples.

#[cfg(feature = "audio")]
mod sink {
    use athena_voice_core::event::{AudioFormat, Event};
    use rodio::buffer::SamplesBuffer;
    use rodio::{OutputStream, Sink};
    use tokio::sync::broadcast;
    use tracing::{debug, warn};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    enum Cmd {
        Play { sample_rate: u32, samples: Vec<f32> },
        Volume(f32),
    }

    pub struct AudioSink {
        rx: broadcast::Receiver<Event>,
    }

    impl AudioSink {
        #[must_use]
        pub fn new(rx: broadcast::Receiver<Event>) -> Self {
            Self { rx }
        }

        /// Plays incoming `AudioChunk` events until the event bus closes.
        ///
        /// A missing output device is non-fatal: the sink logs once and
        /// drains events without playing, so headless hosts behave the same
        /// as hosts with speakers.
        pub async fn run(mut self) -> Result<(), BoxError> {
            let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
            let player = std::thread::Builder::new()
                .name("athena-audio-sink".into())
                .spawn(move || {
                    let Ok((_stream, handle)) = OutputStream::try_default() else {
                        warn!("no audio output device; chunks will be dropped");
                        while cmd_rx.recv().is_ok() {}
                        return;
                    };
                    let sink = match Sink::try_new(&handle) {
                        Ok(sink) => sink,
                        Err(e) => {
                            warn!(error = %e, "audio sink creation failed; chunks will be dropped");
                            while cmd_rx.recv().is_ok() {}
                            return;
                        }
                    };
                    while let Ok(cmd) = cmd_rx.recv() {
                        match cmd {
                            Cmd::Play {
                                sample_rate,
                                samples,
                            } => sink.append(SamplesBuffer::new(1, sample_rate, samples)),
                            Cmd::Volume(level) => sink.set_volume(level),
                        }
                    }
                    sink.sleep_until_end();
                })?;

            loop {
                match self.rx.recv().await {
                    Ok(Event::AudioChunk {
                        format,
                        sample_rate,
                        payload,
                        ..
                    }) => {
                        let samples = match format {
                            AudioFormat::F32le => payload
                                .chunks_exact(4)
                                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect::<Vec<_>>(),
                            AudioFormat::S16le => payload
                                .chunks_exact(2)
                                .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32_768.0)
                                .collect(),
                            AudioFormat::Opus => {
                                debug!("opus playback not wired yet; dropping chunk");
                                continue;
                            }
                        };
                        if cmd_tx
                            .send(Cmd::Play {
                                sample_rate,
                                samples,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Event::VolumeChanged { level, .. }) => {
                        if cmd_tx.send(Cmd::Volume(level)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "audio sink lagged behind the event bus");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            drop(cmd_tx);
            let _ = tokio::task::spawn_blocking(move || player.join()).await;
            Ok(())
        }
    }
}

#[cfg(feature = "audio")]
pub use sink::AudioSink;
