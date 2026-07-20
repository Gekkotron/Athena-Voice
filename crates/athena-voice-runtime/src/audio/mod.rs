//! Audio playback sink for skill-driven chunks.
//!
//! Plan 7: forwards `Event::AudioChunk` to PipeWire for playback.
//! Predicts session ending via barge-in to flush queued audio.

use std::sync::Arc;

use async_trait::async_trait;
use pipewire::{core::Core, properties, spa::buffer::DataType};
use pipewire::{stream::Stream, spa::pod::Pod};
use tokio::sync::broadcast;
use tracing::{debug, error};

use athena_voice_core::event::{AudioFormat, Event};

pub struct AudioSink {
    event_rx: broadcast::Receiver<Event>,
    volume: f32,
}

impl AudioSink {
    #[must_use]
    pub fn new(event_rx: broadcast::Receiver<Event>) -> Self {
        Self {
            event_rx,
            volume: 1.0,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let core = Core::new(None)?;
        let stream = Stream::new(
            &core,
            "athena-sink",
            properties! {
                "media.type" => "audio",
                "media.category" =>"playback",
                "media.role" => "music",
            },
        )?;

        // 48 kHz, 2ch, F32LE for simplicity.
        let params = [Pod::from_bytes(
            &pipewire::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pipewire::spa::param::audio::AudioInfoRaw {
                    format: pipewire::spa::param::audio::AudioFormat::F32LE,
                    rate: 48_000,
                    channels: 2,
                    ..Default::default()
                },
            )?,
        )?];

        stream.connect(
            pipewire::spa::direction::Direction::Output,
            None,
            params,
            pipewire::stream::StreamFlags::AUTOCONNECT
                | pipewire::stream::StreamFlags::INACTIVE,
        )?;

        let mut stream = stream;
        tokio::task::block_in_place(move || {
            while let Ok(event) = self.event_rx.recv() {
                match event {
                    Event::AudioChunk {
                        format,
                        payload,
                        ..
                    } => self.play(&mut stream, format, payload).unwrap(),
                    Event::Volume(level) => self.volume = level.clamp(0.0, 1.5),
                    Event::BargeIn { .. } => {
                        // Flush queued audio on barge-in.
                        stream.flush(false)?;
                    }
                    _ => {} // Ignore other events
                }
            }
            Ok(())
        })
    }

    fn play(
        &self,
        stream: &mut Stream,
        format: AudioFormat,
        payload: Vec<u8>,
    ) -> anyhow::Result<()> {
        match format {
            AudioFormat::S16le => {
                // Convert S16LE → F32LE
                let samples: Vec<f32> = payload
                    .chunks_exact(2)
                    .map(|chunk| {
                        let sample = i16::from_le_bytes(chunk.try_into().unwrap());
                        f32::from(sample) / 32_768.0 * self.volume
                    })
                    .collect();
                let data = Pod::from_bytes(
                    &pipewire::spa::pod::serialize::PodSerializer::serialize(
                        std::io::Cursor::new(Vec::new()),
                        &pipewire::spa::pod::audio::AudioData::new(
                            DataType::F32,
                            48_000,
                            &samples,
                            2,
                        ),
                    )?,
                )?;
                stream.write_pod(&data)?;
            }
            AudioFormat::F32le => {
                // Scale by volume
                let samples: Vec<f32> = payload
                    .chunks_exact(4)
                    .map(|chunk| {
                        let sample = f32::from_le_bytes(chunk.try_into().unwrap());
                        sample * self.volume
                    })
                    .collect();
                let data = Pod::from_bytes(
                    &pipewire::spa::pod::serialize::PodSerializer::serialize(
                        std::io::Cursor::new(Vec::new()),
                        &pipewire::spa::pod::audio::AudioData::new(
                            DataType::F32,
                            48_000,
                            &samples,
                            2,
                        ),
                    )?,
                )?;
                stream.write_pod(&data)?;
            }
            AudioFormat::Opus => {
                // TODO: Opus frame → PCM decoding.
                // Write raw Opus as fallback.
                let data = Pod::from_bytes(
                    &pipewire::spa::pod::serialize::PodSerializer::serialize(
                        std::io::Cursor::new(Vec::new()),
                        &payload,
                    )?,
                )?;
                stream.write_pod(&data)?;
            }
        }
        Ok(())
    }
}