//! Audio playback sink for skill-driven chunks.

#[cfg(feature = "audio")]
mod sink {
    use std::sync::Arc;

    use async_trait::async_trait;
    use pipewire::{core::Core, properties};
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
            // ... rest of the impl
            Ok(())
        }
    }
}

#[cfg(feature = "audio")]
pub use sink::AudioSink;