//! Audio sink lifecycle test — only meaningful with the `audio` feature
//! (`cargo test -p athena-voice-runtime --features audio`). Without an
//! output device (headless CI) the sink must still drain the bus and exit
//! cleanly rather than panic.
#![cfg(feature = "audio")]

use std::time::Duration;

use athena_voice_core::event::{AudioFormat, Event};
use athena_voice_core::ids::SessionId;
use athena_voice_runtime::audio::AudioSink;
use tokio::sync::broadcast;

#[tokio::test(flavor = "multi_thread")]
async fn sink_drains_bus_and_exits_on_close() {
    let (tx, rx) = broadcast::channel(8);
    let task = tokio::spawn(AudioSink::new(rx).run());

    // A quiet 60 ms f32 tone plus a volume change.
    #[allow(clippy::cast_precision_loss)]
    let payload: Vec<u8> = (0..480)
        .flat_map(|i| ((i as f32 * 0.1).sin() * 0.05).to_le_bytes())
        .collect();
    let _ = tx.send(Event::AudioChunk {
        session: SessionId::new_v4(),
        format: AudioFormat::F32le,
        sample_rate: 8_000,
        payload,
    });
    let _ = tx.send(Event::VolumeChanged {
        session: SessionId::new_v4(),
        level: 0.5,
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Closing the bus must terminate the sink.
    drop(tx);
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("sink must exit after the bus closes")
        .expect("sink task must not panic");
    assert!(result.is_ok(), "sink returned an error: {result:?}");
}
