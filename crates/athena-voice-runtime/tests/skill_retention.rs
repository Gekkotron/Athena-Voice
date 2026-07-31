//! Skill retention TTL integration test.
//!
//! Uses the `skills-smoke-test` WASM to verify:
//! - Keys set by a skill are automatically GC'd after the configured TTL
//! - The skill can still access keys before they expire
//! - The storage layer respects the TTL

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use extism::{Manifest, PluginBuilder, Wasm};
use tokio::time::sleep;

use athena_voice_runtime::wasm::host_fns::{AsyncClientPublisher, SkillCtx, host_functions};
use athena_voice_runtime::wasm::registry::{ExtismSkillPlugin, SkillPlugin, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};

const SKILL_NAME: &str = "smoke-test";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skill_retention_ttl_expires_keys() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("SMOKE_TEST_WASM"));
    assert!(wasm_path.exists(), "smoke-test wasm missing");

    // Setup storage
    let store: Arc<dyn Store> = Arc::new(
        SqliteStore::open("sqlite::memory:")
            .await
            .expect("sqlite in-memory store"),
    );

    // Create skill with retention TTL=1s
    let mut config = HashMap::new();
    config.insert("greeting".into(), "bonjour".into());

    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: Arc::new(AsyncClientPublisher(
            rumqttc::AsyncClient::new(rumqttc::MqttOptions::new("test", "127.0.0.1", 1883), 8).0,
        )),
        http_allowlist: vec!["smoke.local".into()],
        mqtt_publish_allowlist: Vec::new(),
        config,
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        retention_gc_after_sec: Some(1), // 1s TTL
        event_bus: tokio::sync::broadcast::channel(8).0,
        config_file: None,
    };

    let manifest = Manifest::new([Wasm::file(&wasm_path)]);
    let plugin = PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_functions(host_functions(ctx.clone()))
        .build()
        .expect("build extism plugin");
    let plugin: Arc<Mutex<dyn SkillPlugin>> =
        Arc::new(Mutex::new(ExtismSkillPlugin::with_ctx(plugin, ctx)));

    let registry = SkillRegistry::new();
    registry
        .install(SKILL_NAME, plugin, &["fr".into()])
        .expect("install skill");
    let registry = Arc::new(registry);

    // Simulate a skill dispatch (this will set a key with automatic
    // timestamp). Dispatch is blocking (the host fns bridge async work with
    // `block_on`), so it must run off the test's async runtime thread.
    let intent = athena_voice_skill_sdk::Intent {
        name: "time.query".into(),
        slots: Default::default(),
        locale: String::new(),
    };
    let reg = registry.clone();
    tokio::task::spawn_blocking(move || reg.dispatch(SKILL_NAME, intent))
        .await
        .expect("join dispatch")
        .expect("dispatch");

    // Verify the key exists immediately after setting.
    let value = store
        .skill_kv_get(SKILL_NAME, "last_intent")
        .await
        .expect("get failed")
        .expect("key should exist");
    assert_eq!(value.as_slice(), b"time.query");

    // Wait for TTL to expire
    sleep(Duration::from_secs(2)).await;

    // Simulate another dispatch — the unknown intent writes no state itself
    // but still triggers the retention GC pass.
    let intent = athena_voice_skill_sdk::Intent {
        name: "other.query".into(),
        slots: Default::default(),
        locale: String::new(),
    };
    let reg = registry.clone();
    let _ = tokio::task::spawn_blocking(move || reg.dispatch(SKILL_NAME, intent))
        .await
        .expect("join dispatch");

    // GC runs on a spawned task — poll until it lands.
    let mut gone = false;
    for _ in 0..40 {
        if store
            .skill_kv_get(SKILL_NAME, "last_intent")
            .await
            .expect("get failed")
            .is_none()
        {
            gone = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "key should have been GC'd");
}
