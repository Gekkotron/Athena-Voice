//! Runtime demo: loads skills, processes intents.
use std::time::Duration;

use athena_voice_core::types::Intent;
use athena_voice_runtime::{Config, Runtime, SkillDeps};
use athena_voice_storage::SqliteStore;
use rumqttc::MqttOptions;
use tokio::sync::broadcast;

use athena_voice_providers::ProviderFactory;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    // Setup
    let mqtt_cfg = MqttOptions::new("demo-client", "127.0.0.1", 1883);
    let factory = ProviderFactory::simple();
    let mut runtime = Runtime::spawn(mqtt_cfg, factory)?;
    let store = SqliteStore::open("sqlite::memory:").await?;
    
    // Load skills dir
    let skills_dir = "./skills";
    let skill_deps = SkillDeps {
        store: store.into(),
        mqtt: runtime.sessions.dispatcher()?.mqtt_client(),
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        locales: vec!["fr".into()],
        per_skill: Default::default(),
        event_tx: Some(runtime.event_bus.clone()),
        audio_event_tx: runtime.event_bus.clone(),
    };
    
    // Hot-reload observer
    let mut events = runtime.event_bus.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            println!("📡 Event: {:?}", event);
        }
    });
    
    // Test: dispatch intents
    runtime.sessions.dispatcher()?.call(
        uuid::Uuid::new_v4().into(),
        Intent::new("time.query", vec![])?,
    ).await?;
    
    runtime.sessions.dispatcher()?.call(
        uuid::Uuid::new_v4().into(),
        Intent::new("audio.play", vec![])?,
    ).await?;
    
    // Sleep and shutdown
    tokio::time::sleep(Duration::from_secs(5)).await;
    runtime.shutdown().await;
    Ok(())
}