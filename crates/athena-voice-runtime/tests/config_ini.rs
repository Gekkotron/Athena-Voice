//! Test INI config file support.

use std::collections::HashMap;

use athena_voice_runtime::{Config, Runtime, SkillDeps, SkillRegistry};
use athena_voice_storage::SqliteStore;
use rumqttc::MqttOptions;
use tempfile::NamedTempFile;
use tokio::sync::broadcast;

use athena_voice_providers::ProviderFactory;

#[tokio::test(flavor = "multi_thread")]
async fn ini_config_file_roundtrip() {
    let _ = tracing_subscriber::fmt::try_init();

    // Setup
    let mqtt_cfg = MqttOptions::new("test-client", "127.0.0.1", 1883);
    let factory = ProviderFactory::simple();
    let runtime = Runtime::spawn(mqtt_cfg, factory).unwrap();
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    
    // Create INI config
    let ini_content = "[audio]
speed = fast";
    let ini_file = NamedTempFile::new().unwrap();
    std::fs::write(ini_file.path(), ini_content).unwrap();

    // Create SkillsDeps with config_file
    let mut per_skill = HashMap::new();
    per_skill.insert("audio-test".to_string(), 
        athena_voice_runtime::SkillConfig {
            config_file: Some(ini_file.path().to_string_lossy().into_owned()),
            ..Default::default()
        }
    );
    
    let skill_deps = SkillDeps {
        store: store.into(),
        mqtt: runtime.sessions.dispatcher().unwrap().mqtt_client(),
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        locales: vec!["fr".into()],
        per_skill,
        event_tx: None,
        audio_event_tx: runtime.event_bus.subscribe().sender(),
    };
    
    // Load skill
    let skills_dir = NamedTempFile::new().unwrap().path().to_path_buf();
    let registry = SkillRegistry::load_dir(&skills_dir, skill_deps).unwrap();
    
    // Build skill .wasm
    let wasm_path = "./skills-audio-test/target/wasm32-wasip1/debug/skills_audio_test.wasm";
    std::fs::create_dir_all(skills_dir.parent().unwrap()).unwrap();
    std::fs::copy(
        "./target/wasm32-wasip1/debug/skills_audio_test.wasm",
        skills_dir.join("audio-test.wasm"),
    ).unwrap();
    
    // Reload registry
    registry.load_dir(&skills_dir, skill_deps).unwrap();
    
    // Dispatch test intent
    let response = registry.dispatch("audio-test", 
        athena_voice_core::types::Intent::new("audio.test", vec![]).unwrap()
    ).unwrap();
    
    if let SkillResponse::Speak(text) = response {
        assert!(text.contains("fast"));
    } else {
        panic!("Unexpected response: {:?}", response);
    }
}