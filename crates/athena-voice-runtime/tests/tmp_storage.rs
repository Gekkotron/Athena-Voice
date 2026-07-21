//! Test skill-local transient storage.

use athena_voice_runtime::{Runtime, SkillDeps, SkillRegistry};
use athena_voice_storage::SqliteStore;
use rumqttc::MqttOptions;
use tempfile::NamedTempFile;
use uuid::Uuid;

use athena_voice_providers::ProviderFactory;

#[tokio::test(flavor = "multi_thread")]
async fn tmp_storage_roundtrip() {
    let _ = tracing_subscriber::fmt::try_init();

    // Setup
    let mqtt_cfg = MqttOptions::new("test-tmp", "127.0.0.1", 1883);
    let factory = ProviderFactory::simple();
    let runtime = Runtime::spawn(mqtt_cfg, factory).unwrap();
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    
    // Create test skill
    let skills_dir = NamedTempFile::new().unwrap().path().to_path_buf();
    std::fs::create_dir_all(&skills_dir).unwrap();
    
    let skill_deps = SkillDeps {
        store: store.into(),
        mqtt: runtime.sessions.dispatcher().unwrap().mqtt_client(),
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        locales: vec!["fr".into()],
        per_skill: Default::default(),
        event_tx: None,
        audio_event_tx: runtime.event_bus.subscribe().sender(),
    };
    
    // Load skill
    let registry = SkillRegistry::load_dir(&skills_dir, skill_deps).unwrap();
    
    // Build skill .wasm
    let wasm_path = "./skills-tmp-test/target/wasm32-wasip1/debug/skills_tmp_test.wasm";
    std::fs::copy(wasm_path, skills_dir.join("tmp-test.wasm")).unwrap();
    
    // Test: set/read key
    let response = registry.dispatch("tmp-test", 
        athena_voice_core::types::Intent::new("tmp.test", vec![
            athena_voice_core::types::Slot::String("test-key".into()),
            athena_voice_core::types::Slot::String("test-val".into()),
        ]).unwrap()
    ).unwrap();
    
    if let athena_voice_skill_sdk::SkillResponse::Speak(text) = response {
        assert!(text.contains("found"));
    } else {
        panic!("Unexpected response: {:?}", response);
    }
}