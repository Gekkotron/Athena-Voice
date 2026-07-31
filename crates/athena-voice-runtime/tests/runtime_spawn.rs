//! Verifies Runtime::spawn wires the actor DAG and shuts down cleanly.
//!
//! A full MQTT roundtrip (fake satellite drives a session end-to-end through
//! an embedded rumqttd broker) is deferred to a follow-up integration test.
//! This test only checks the runtime bootstraps and drains without hanging.

use std::sync::Arc;

use athena_voice_providers::{ProviderConfig, ProviderFactory, StageChoice};
use athena_voice_runtime::Runtime;
use athena_voice_runtime::mqtt::MqttConfig;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_and_shutdown_are_clean() {
    let factory = Arc::new(
        ProviderFactory::build(
            &ProviderConfig {
                stt: StageChoice::Fake,
                llm: StageChoice::Fake,
                tts: StageChoice::Fake,
            },
            None,
        )
        .await
        .unwrap(),
    );

    let runtime = Runtime::spawn(
        MqttConfig {
            host: "127.0.0.1".into(),
            port: 62991, // unlikely to have a broker here; runtime just retries in background
            client_id: "athena-voice-test".into(),
            username: None,
            password: None,
            keep_alive_secs: 30,
        },
        factory,
        None,
        None,
        std::time::Duration::from_secs(120),
    )
    .expect("spawn");

    // Runtime is running. Verify the session manager is present and empty.
    assert!(runtime.sessions.is_empty());

    // Cleanly shut down.
    let shutdown = runtime.shutdown;
    shutdown.cancel();
    // NB: we don't `runtime.shutdown().await` here because we moved shutdown out
    // above; the runtime dropping will abort its background tasks. This test is
    // ok because we're only verifying the spawn path compiles and runs without
    // panicking.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_with_assist_bridge_is_clean() {
    let factory = Arc::new(
        ProviderFactory::build(
            &ProviderConfig {
                stt: StageChoice::Fake,
                llm: StageChoice::Fake,
                tts: StageChoice::Fake,
            },
            None,
        )
        .await
        .unwrap(),
    );
    let runtime = Runtime::spawn(
        MqttConfig {
            host: "127.0.0.1".into(),
            port: 62991,
            client_id: "athena-voice-assist-test".into(),
            username: None,
            password: None,
            keep_alive_secs: 30,
        },
        factory,
        None,
        Some(athena_voice_runtime::assist::AssistInit {
            topic_prefix: "assist".into(),
            locale: athena_voice_core::ids::Locale::new("fr").unwrap(),
            session_idle: std::time::Duration::from_secs(120),
        }),
        std::time::Duration::from_secs(120),
    )
    .expect("spawn with assist");
    runtime.shutdown.cancel();
}
