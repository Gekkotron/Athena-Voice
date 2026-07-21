//! Integration test for skill-driven audio playback.
//! Plan 7: dispatches "joue un son" → asserts AudioChunk emitted.

use std::time::Duration;

use athena_voice_core::event::{AudioFormat, Event};
use athena_voice_runtime::{AudioSink, Config, Runtime, SkillDispatcher};
use athena_voice_storage::SqliteStore;
use rumqttc::{AsyncClient, MqttOptions};
use tokio::sync::broadcast;
use uuid::Uuid;

use athena_voice_providers::ProviderFactory;

#[tokio::test(flavor = "multi_thread")]
async fn skill_driven_playback_emits_audio_chunk() {
    let _ = tracing_subscriber::fmt::try_init();

    // Setup
    let mqtt_cfg = MqttOptions::new("test-client", "127.0.0.1", 1883);
    let factory = ProviderFactory::simple();
    let runtime = Runtime::spawn(mqtt_cfg, factory).unwrap();
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    let dispatcher = runtime.sessions.dispatcher().unwrap();
    let session = Uuid::new_v4().into();

    // Audio capture bus
    let audio_rx = runtime.event_bus.subscribe();

    // Test: dispatch a "mets le volume à 70%" (audio.volume) intent
    dispatcher
        .call(
            session,
            athena_voice_core::types::Intent::new(
                "audio.volume",
                vec![athena_voice_core::types::Slot::Float(0.7)],
            ).unwrap(),
        )
        .await
        .unwrap();

    // Set volume and poll for AudioChunk event
    dispatcher
        .call(
            session,
            athena_voice_core::types::Intent::new(
                "audio.volume",
                vec![athena_voice_core::types::Slot::Float(0.7)],
            ).unwrap(),
        )
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        let mut volume_adjusted = false;
        for _ in 0..20 {
            if let Ok(event) = audio_rx.recv().await {
                match event {
                    Event::AudioChunk { format, payload, .. } => {
                        assert_eq!(format, AudioFormat::F32le);
                        assert!(payload.len() >= 4); // At least one f32
                        return;
                    }
                    Event::Volume(level) => {
                        assert!((level - 0.7).abs() < f32::EPSILON);
                        volume_adjusted = true;
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(volume_adjusted, "Volume event not received");
        panic!("No AudioChunk received");
    }).await.unwrap();
}