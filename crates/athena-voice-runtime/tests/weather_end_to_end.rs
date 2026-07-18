//! End-to-end weather skill dispatch (Task E).
//!
//! Loads the `skills-weather` WASM (path via `WEATHER_TEST_WASM`), wires it
//! into a `SkillRegistry` + `SkillDispatcher`, points the skill at two
//! `wiremock` HTTP servers (one for geocoding, one for forecast) via the
//! `geocoding_base_url` / `forecast_base_url` per-skill config keys, feeds a
//! French final transcript, and asserts:
//!
//! - Known city → forecast is fetched and the runtime speaks the expected
//!   Celsius line.
//! - `weather.tomorrow` → daily min/max are read from `daily[0]`.
//! - Unknown city → geocoding returns an empty `results` array, the skill
//!   apologises AND the forecast mock endpoint is never hit.

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

const SKILL_NAME: &str = "weather";

/// The skill never publishes MQTT — this stub exists to satisfy `SkillCtx`.
struct NoopMqtt;

#[async_trait]
impl MqttPublisher for NoopMqtt {
    async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
        Ok(())
    }
}

/// Boots a fresh pipeline for one utterance and returns the concatenated TTS
/// output.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
async fn drive_utterance(
    wasm_path: &std::path::Path,
    utterance: &str,
    default_city: &str,
    geocoding_base_url: &str,
    forecast_base_url: &str,
) -> Vec<String> {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let http = reqwest::Client::new();

    let mut config = HashMap::new();
    config.insert("default_city".into(), default_city.into());
    config.insert("units".into(), "celsius".into());
    config.insert("geocoding_base_url".into(), geocoding_base_url.into());
    config.insert("forecast_base_url".into(), forecast_base_url.into());

    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: Arc::new(NoopMqtt) as Arc<dyn MqttPublisher>,
        // Wiremock lives on 127.0.0.1 — that's what the skill will hit.
        http_allowlist: vec!["127.0.0.1".into()],
        mqtt_publish_allowlist: Vec::new(),
        config,
        tokio: tokio::runtime::Handle::current(),
        http,
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
        .expect("install weather skill");
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
    // Weather HTTP calls take longer than the timer/home pipelines. Wait a
    // bit longer for the first chunk, then drain quickly.
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

    chunks
}

/// TTS chunks arrive one word at a time; join with spaces so multi-word
/// assertions still work.
fn joined(chunks: &[String]) -> String {
    chunks.join(" ")
}

fn geocoding_ok(name: &str, lat: f64, lon: f64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "results": [
            { "name": name, "latitude": lat, "longitude": lon, "country": "France" }
        ]
    }))
}

fn geocoding_empty() -> ResponseTemplate {
    // Open-Meteo omits `results` entirely when no city matches — the skill
    // must handle both an empty array and a missing field.
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "generationtime_ms": 0.5
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn weather_now_at_city_speaks_current_temperature() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("WEATHER_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "weather wasm missing at {}",
        wasm_path.display()
    );

    let geo_server = MockServer::start().await;
    let fc_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/search"))
        .and(query_param("name", "Lyon"))
        .and(query_param("language", "fr"))
        .and(query_param("count", "1"))
        .respond_with(geocoding_ok("Lyon", 45.75, 4.85))
        .expect(1)
        .mount(&geo_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/forecast"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "current": { "temperature_2m": 18.0, "weather_code": 1 },
            "daily": {
                "time": ["2026-07-19"],
                "temperature_2m_max": [22.0],
                "temperature_2m_min": [12.0],
                "weather_code": [1]
            }
        })))
        .expect(1)
        .mount(&fc_server)
        .await;

    let chunks = drive_utterance(
        &wasm_path,
        "quel temps fait-il à Lyon",
        "Paris",
        &geo_server.uri(),
        &fc_server.uri(),
    )
    .await;

    let joined = joined(&chunks);
    assert!(
        joined.contains("il fait 18 degrés à Lyon, quelques nuages"),
        "expected the 'il fait 18 degrés à Lyon, quelques nuages' line; got {chunks:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn weather_tomorrow_uses_default_city_and_daily_extremes() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("WEATHER_TEST_WASM"));
    assert!(wasm_path.exists());

    let geo_server = MockServer::start().await;
    let fc_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/search"))
        .and(query_param("name", "Paris"))
        .respond_with(geocoding_ok("Paris", 48.85, 2.35))
        .expect(1)
        .mount(&geo_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/forecast"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "current": { "temperature_2m": 14.0, "weather_code": 61 },
            "daily": {
                "time": ["2026-07-19"],
                "temperature_2m_max": [15.0],
                "temperature_2m_min": [8.0],
                "weather_code": [61]
            }
        })))
        .expect(1)
        .mount(&fc_server)
        .await;

    let chunks = drive_utterance(
        &wasm_path,
        "quel temps fera-t-il demain",
        "Paris",
        &geo_server.uri(),
        &fc_server.uri(),
    )
    .await;

    let joined = joined(&chunks);
    assert!(
        joined.contains("demain à Paris, il fera entre 8 et 15 degrés avec de la pluie"),
        "expected the tomorrow line for Paris; got {chunks:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_city_apologises_without_calling_forecast() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("WEATHER_TEST_WASM"));
    assert!(wasm_path.exists());

    let geo_server = MockServer::start().await;
    let fc_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/search"))
        .and(query_param("name", "Zzzz"))
        .respond_with(geocoding_empty())
        .expect(1)
        .mount(&geo_server)
        .await;

    // Forecast must not be called for unknown cities.
    Mock::given(method("GET"))
        .and(path("/v1/forecast"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&fc_server)
        .await;

    let chunks = drive_utterance(
        &wasm_path,
        "quel temps fait-il à Zzzz",
        "Paris",
        &geo_server.uri(),
        &fc_server.uri(),
    )
    .await;

    let joined = joined(&chunks);
    assert!(
        joined.contains("désolé, je ne trouve pas Zzzz"),
        "expected apology mentioning Zzzz; got {chunks:?}"
    );
    // The `.expect(0)` on the forecast mock is verified on drop — if any hit
    // came through, wiremock panics as this test unwinds.
}
