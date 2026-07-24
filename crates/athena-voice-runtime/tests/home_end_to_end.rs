//! End-to-end home-automation skill dispatch (Task D).
//!
//! Loads the `skills-home` WASM (path via `HOME_TEST_WASM`), wires it into a
//! `SkillRegistry` + `SkillDispatcher` with a capturing `MqttPublisher`
//! backend, feeds a French final transcript, and asserts:
//!
//! - Known entity → the matching `set_topic` is published with the expected
//!   payload AND the runtime speaks `"d'accord"`.
//! - Unknown entity → NO publish AND the runtime speaks a `"désolé"` line.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use extism::{Manifest, PluginBuilder, Wasm};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Tts;
use athena_voice_core::types::Transcript;
use athena_voice_providers::testing::fake_tts::FakeTts;

use athena_voice_runtime::intent::IntentMatcher;
use athena_voice_runtime::pipeline::router::{RouterDeps, spawn_router};
use athena_voice_runtime::pipeline::tts::spawn_tts;
use athena_voice_runtime::wasm::dispatcher::SkillDispatcher;
use athena_voice_runtime::wasm::host_fns::{MqttPublisher, SkillCtx, host_functions};
use athena_voice_runtime::wasm::registry::{ExtismSkillPlugin, SkillPlugin, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};

const SKILL_NAME: &str = "home";

/// In-memory publisher that records every `(topic, payload)` and pretends the
/// broker acked.
#[derive(Default)]
struct CaptureMqtt {
    captured: Mutex<Vec<(String, Vec<u8>)>>,
}

#[async_trait]
impl MqttPublisher for CaptureMqtt {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
        self.captured.lock().unwrap().push((topic, payload));
        Ok(())
    }
}

impl CaptureMqtt {
    fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        self.captured.lock().unwrap().clone()
    }
}

/// Boots a fresh pipeline for one utterance and returns the captured MQTT
/// publishes plus the TTS chunks emitted.
#[allow(clippy::too_many_lines)]
async fn drive_utterance(
    wasm_path: &std::path::Path,
    utterance: &str,
) -> (Vec<(String, Vec<u8>)>, Vec<String>) {
    let capture: Arc<CaptureMqtt> = Arc::new(CaptureMqtt::default());

    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let http = reqwest::Client::new();

    // Two entities: one light in the salon, one switch (`prise`) in the bureau.
    let entities = serde_json::json!([
        {
            "name": "lumière du salon",
            "room": "salon",
            "kind": "light",
            "set_topic": "home/salon/light/set",
            "on_payload": "ON",
            "off_payload": "OFF",
        },
        {
            "name": "prise du bureau",
            "room": "bureau",
            "kind": "switch",
            "set_topic": "home/bureau/switch/set",
            "on_payload": "ON",
            "off_payload": "OFF",
        },
    ])
    .to_string();
    let mut config = HashMap::new();
    config.insert("entities".to_string(), entities);

    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: capture.clone() as Arc<dyn MqttPublisher>,
        http_allowlist: Vec::new(),
        // Real MQTT topics live outside `athena/skills/home/*` — the
        // allowlist is what lets the skill reach them.
        mqtt_publish_allowlist: vec!["home/+/light/set".into(), "home/+/switch/set".into()],
        config,
        tokio: tokio::runtime::Handle::current(),
        http,
        retention_gc_after_sec: None,
        event_bus: tokio::sync::broadcast::channel(8).0,
        config_file: None,
    };

    let manifest = Manifest::new([Wasm::file(wasm_path)]);
    let plugin = PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_functions(host_functions(ctx))
        .build()
        .expect("build extism plugin");
    let plugin: Arc<Mutex<dyn SkillPlugin>> = Arc::new(Mutex::new(ExtismSkillPlugin::new(plugin)));

    let registry = SkillRegistry::new();
    registry
        .install(SKILL_NAME, plugin, &["fr".into()])
        .expect("install home skill");
    let rules = registry.patterns_handle();
    let registry = Arc::new(registry);

    let (ev_tx, _ev_rx) = broadcast::channel::<Event>(128);
    let cancel = CancellationToken::new();
    let session = SessionId::new_v4();
    let locale = Locale::new("fr").unwrap();

    let (dispatcher_handle, dispatcher_task) =
        SkillDispatcher::spawn(registry.clone(), ev_tx.clone(), cancel.clone());

    let (tok_tx, tok_rx) = mpsc::channel::<String>(16);
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<Bytes>(32);
    let tts: Arc<dyn Tts> = Arc::new(FakeTts::new());
    let tts_task = spawn_tts(
        session,
        locale.clone(),
        tts,
        tok_rx,
        chunk_tx,
        ev_tx.clone(),
        cancel.clone(),
    );

    let (llm_tx, _llm_rx) = mpsc::channel::<String>(4);
    let (t_tx, t_rx) = mpsc::channel::<Transcript>(4);
    let router_deps = RouterDeps {
        llm_tx,
        tts_tok_tx: tok_tx.clone(),
        event_tx: ev_tx.clone(),
        session,
        locale: locale.clone(),
        matcher: Arc::new(IntentMatcher::new()),
        rules,
        dispatcher: Some(dispatcher_handle.clone()),
    };
    let router_task = spawn_router(t_rx, router_deps, cancel.clone());

    t_tx.send(Transcript {
        text: utterance.into(),
        is_final: true,
        confidence: None,
    })
    .await
    .expect("send transcript");

    // Drain chunks for a short window — the pipeline is one-shot per
    // utterance and finishes well within a second.
    let mut chunks: Vec<String> = Vec::new();
    while let Ok(Some(chunk)) = timeout(Duration::from_millis(1500), chunk_rx.recv()).await {
        chunks.push(String::from_utf8_lossy(&chunk).into_owned());
    }

    drop(t_tx);
    drop(tok_tx);
    cancel.cancel();
    let _ = router_task.await;
    let _ = tts_task.await;
    drop(dispatcher_handle);
    let _ = dispatcher_task.await;

    (capture.snapshot(), chunks)
}

fn joined(chunks: &[String]) -> String {
    chunks.join("")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn allume_lumiere_du_salon_publishes_and_confirms() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("HOME_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "home wasm missing at {}",
        wasm_path.display()
    );

    let (published, chunks) = drive_utterance(&wasm_path, "allume la lumière du salon").await;

    assert_eq!(
        published.len(),
        1,
        "expected exactly one publish; got {published:?}"
    );
    assert_eq!(published[0].0, "home/salon/light/set");
    assert_eq!(published[0].1, b"ON");
    assert!(
        joined(&chunks).contains("d'accord"),
        "expected TTS to contain 'd'accord', got {chunks:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eteins_prise_du_bureau_publishes_off_and_confirms() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("HOME_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "home wasm missing at {}",
        wasm_path.display()
    );

    let (published, chunks) = drive_utterance(&wasm_path, "éteins la prise du bureau").await;

    assert_eq!(
        published.len(),
        1,
        "expected exactly one publish; got {published:?}"
    );
    assert_eq!(published[0].0, "home/bureau/switch/set");
    assert_eq!(published[0].1, b"OFF");
    assert!(
        joined(&chunks).contains("d'accord"),
        "expected TTS to contain 'd'accord', got {chunks:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_entity_apologises_and_does_not_publish() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("HOME_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "home wasm missing at {}",
        wasm_path.display()
    );

    let (published, chunks) = drive_utterance(&wasm_path, "allume la lumière de la piscine").await;

    assert!(
        published.is_empty(),
        "expected no publish for unknown entity; got {published:?}"
    );
    assert!(
        joined(&chunks).contains("désolé"),
        "expected TTS to contain 'désolé', got {chunks:?}"
    );
}
