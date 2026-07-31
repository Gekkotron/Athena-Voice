//! End-to-end Jeedom sensor skill dispatch.
//!
//! Loads the `skills-jeedom` WASM (path via `JEEDOM_TEST_WASM`), points it
//! at a `wiremock` server standing in for the Jeedom box, feeds French
//! final transcripts, and asserts:
//!
//! - Known sensor → jeeApi is called with the api key and cmd id, and the
//!   runtime speaks the value with its unit.
//! - Unknown sensor → apology AND the API is never hit.
//! - Jeedom unreachable (HTTP 500) → spoken failure line.

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
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

const SKILL_NAME: &str = "jeedom";
const SENSORS: &str = r#"[
    {"name":"température du salon","id":123,"unit":"degrés"},
    {"name":"humidité de la chambre","id":456,"unit":"pourcent"}
]"#;

struct NoopMqtt;

#[async_trait]
impl MqttPublisher for NoopMqtt {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

/// Boots a fresh pipeline for one utterance and returns the joined TTS text.
#[allow(clippy::too_many_lines)]
async fn drive_utterance(base_url: &str, utterance: &str) -> String {
    let wasm_path = PathBuf::from(env!("JEEDOM_TEST_WASM"));
    assert!(wasm_path.exists(), "jeedom wasm missing");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());

    let mut config = HashMap::new();
    config.insert("base_url".into(), base_url.to_string());
    config.insert("api_key".into(), "test-key".into());
    config.insert("sensors".into(), SENSORS.into());

    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: Arc::new(NoopMqtt) as Arc<dyn MqttPublisher>,
        http_allowlist: vec!["127.0.0.1".into()],
        mqtt_publish_allowlist: Vec::new(),
        config,
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        retention_gc_after_sec: None,
        event_bus: tokio::sync::broadcast::channel(8).0,
        config_file: None,
    };

    let manifest = Manifest::new([Wasm::file(&wasm_path)]);
    let plugin = PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_functions(host_functions(ctx))
        .build()
        .expect("build extism plugin");
    let plugin: Arc<Mutex<dyn SkillPlugin>> = Arc::new(Mutex::new(ExtismSkillPlugin::new(plugin)));

    let registry = SkillRegistry::new();
    registry
        .install(SKILL_NAME, plugin, &["fr".into()])
        .expect("install jeedom skill");
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

    let mut chunks: Vec<String> = Vec::new();
    while let Ok(Some(chunk)) = timeout(Duration::from_millis(3000), chunk_rx.recv()).await {
        chunks.push(String::from_utf8_lossy(&chunk).into_owned());
    }

    drop(t_tx);
    drop(tok_tx);
    cancel.cancel();
    let _ = router_task.await;
    let _ = tts_task.await;
    drop(dispatcher_handle);
    let _ = dispatcher_task.await;

    chunks.join(" ")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn known_sensor_speaks_value_with_unit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/core/api/jeeApi.php"))
        .and(query_param("apikey", "test-key"))
        .and(query_param("type", "cmd"))
        .and(query_param("id", "123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("21.5"))
        .expect(1)
        .mount(&server)
        .await;

    let spoken = drive_utterance(&server.uri(), "donne-moi la température du salon").await;
    assert!(
        spoken.contains("21.5") && spoken.contains("degrés"),
        "spoken: {spoken:?}"
    );
    assert!(
        spoken.contains("température du salon"),
        "spoken: {spoken:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn natural_phrasing_with_stt_slip_reads_the_sensor() {
    // Real mic interaction: "quelle est la température du salon" transcribed
    // as "…du salaud". The per-sensor literal rule must catch it.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/core/api/jeeApi.php"))
        .and(query_param("id", "123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("21.5"))
        .expect(1)
        .mount(&server)
        .await;

    let spoken = drive_utterance(&server.uri(), "Quelle est la température du salaud").await;
    assert!(
        spoken.contains("21.5") && spoken.contains("température du salon"),
        "spoken: {spoken:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_sensor_apologises_without_calling_jeedom() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/core/api/jeeApi.php"))
        .respond_with(ResponseTemplate::new(200).set_body_string("0"))
        .expect(0)
        .mount(&server)
        .await;

    let spoken = drive_utterance(&server.uri(), "capteur pression du garage").await;
    assert!(spoken.contains("désolé"), "spoken: {spoken:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jeedom_down_speaks_failure_line() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/core/api/jeeApi.php"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let spoken = drive_utterance(&server.uri(), "capteur humidité de la chambre").await;
    assert!(spoken.contains("joindre Jeedom"), "spoken: {spoken:?}");
}
