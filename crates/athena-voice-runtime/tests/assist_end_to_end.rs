//! End-to-end assist bridge test: a REAL WASM skill answers a text question
//! sent over the bridge's device actor, with no MQTT broker involved.
//!
//! Loads the `skills-smoke-test` WASM built by `build.rs` (path exposed via
//! the `SMOKE_TEST_WASM` env var), wires it into a `SkillRegistry` +
//! `SkillDispatcher`, feeds a French time question through `AssistBridge`,
//! and asserts:
//!
//! - The skill's real French speech is published as `{"text": ...}` on
//!   `assist/tts/{device}`.
//! - The loader status sequence (`in progress` before the answer, `done`
//!   at/after it) is respected.
//! - A second question sent back-to-back (barge-in) still settles into a
//!   quiescent state with at least one `done` status.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use extism::{Manifest, PluginBuilder, Wasm};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use athena_voice_core::ids::Locale;
use athena_voice_providers::{ProviderConfig, ProviderFactory, StageChoice};

use athena_voice_runtime::assist::bridge::{AssistBridge, AssistDeps, AssistInit};
use athena_voice_runtime::intent::IntentMatcher;
use athena_voice_runtime::mqtt::{MqttClient, MqttConfig};
use athena_voice_runtime::wasm::dispatcher::SkillDispatcher;
use athena_voice_runtime::wasm::host_fns::{
    AsyncClientPublisher, MqttPublisher, SkillCtx, host_functions,
};
use athena_voice_runtime::wasm::registry::{ExtismSkillPlugin, SkillPlugin, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};

/// The skill name must match the ACL prefix baked into the smoke skill
/// (`athena/skills/smoke-test/…`) so `mqtt_publish` succeeds; the guest ABI
/// derives ACL from `SkillCtx.name`, not the wasm file stem.
const SKILL_NAME: &str = "smoke-test";

/// Records publishes and wakes waiters. Duplicated from
/// `src/assist/bridge.rs`'s test module: test files can't import each
/// other's `#[cfg(test)]` modules.
struct RecordingPublisher {
    published: Mutex<Vec<(String, String)>>,
    notify: tokio::sync::Notify,
}

impl RecordingPublisher {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            published: Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        })
    }

    /// Waits (5 s cap) until `pred` holds over the published list.
    async fn wait_for(&self, pred: impl Fn(&[(String, String)]) -> bool) {
        timeout(Duration::from_secs(5), async {
            loop {
                if pred(&self.published.lock().unwrap()) {
                    return;
                }
                self.notify.notified().await;
            }
        })
        .await
        .expect("publisher wait timed out");
    }
}

#[async_trait::async_trait]
impl MqttPublisher for RecordingPublisher {
    async fn publish(&self, topic: String, payload: Vec<u8>) -> Result<(), String> {
        self.published
            .lock()
            .unwrap()
            .push((topic, String::from_utf8_lossy(&payload).into_owned()));
        self.notify.notify_waiters();
        Ok(())
    }
}

/// Builds the skill registry (backed by the real smoke-test WASM), the
/// dispatcher, and the intent matcher/rules exactly as
/// `tests/en_end_to_end.rs` does, then wires an `AssistBridge` on top with
/// the given publisher. `llm` is forced to `StageChoice::None`: the smoke
/// skill must answer every question in these tests, so the LLM must never
/// be reached.
async fn build_bridge(publisher: Arc<RecordingPublisher>) -> Arc<AssistBridge> {
    let wasm_path = PathBuf::from(env!("SMOKE_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "smoke-test wasm missing at {}: build.rs should have produced it",
        wasm_path.display()
    );

    // ---------- Skill runtime deps ----------
    let store: Arc<dyn Store> = Arc::new(
        SqliteStore::open("sqlite::memory:")
            .await
            .expect("sqlite in-memory store"),
    );
    // Bogus broker: the client never actually connects, but publishes just
    // enqueue in-memory (queue depth 128) so the smoke skill's single
    // `mqtt_publish` call returns MQTT_OK without touching the network.
    let mqtt = MqttClient::connect(MqttConfig {
        host: "127.0.0.1".into(),
        port: 62992,
        client_id: "athena-voice-assist-test".into(),
        username: None,
        password: None,
        keep_alive_secs: 30,
    })
    .expect("mqtt client");
    let http = reqwest::Client::new();

    let mut config = HashMap::new();
    config.insert("greeting".into(), "bonjour".into());
    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: Arc::new(AsyncClientPublisher(mqtt.tx.clone())),
        http_allowlist: vec!["smoke.local".into()],
        mqtt_publish_allowlist: Vec::new(),
        config,
        tokio: tokio::runtime::Handle::current(),
        http,
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
        .expect("install smoke skill");
    let rules = registry.patterns_handle();
    let registry = Arc::new(registry);

    let (ev_tx, _ev_rx) = broadcast::channel(128);
    let cancel = CancellationToken::new();
    let (dispatcher_handle, _dispatcher_task) =
        SkillDispatcher::spawn(registry.clone(), ev_tx.clone(), cancel.clone());

    // LLM is deliberately unreachable: the smoke skill must answer every
    // question in these tests, so a fallback to the LLM would be a bug.
    let factory = ProviderFactory::build(
        &ProviderConfig {
            stt: StageChoice::Fake,
            llm: StageChoice::None,
            tts: StageChoice::Fake,
        },
        None,
    )
    .await
    .expect("provider factory");
    let factory = Arc::new(factory);

    AssistBridge::new(
        AssistInit {
            topic_prefix: "assist".into(),
            locale: Locale::new("fr").unwrap(),
            session_idle: Duration::from_secs(120),
        },
        AssistDeps {
            publisher,
            factory,
            matcher: Arc::new(IntentMatcher::new()),
            rules,
            dispatcher: Some(dispatcher_handle),
            event_bus: ev_tx,
            shutdown: cancel,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn french_time_question_answered_as_text() {
    let publisher = RecordingPublisher::new();
    let bridge = build_bridge(publisher.clone()).await;

    assert!(bridge.handle(
        "assist/transcription/pixel",
        br#"{"text": "quelle heure est-il"}"#
    ));

    // Wait for BOTH the tts answer and the done status: `done` is published
    // in a later await than the tts publish (publish_answer → then
    // publish_status), so waiting on the tts publish alone races the
    // `done`-status lookup below under CI contention.
    publisher
        .wait_for(|p| {
            p.iter().any(|(t, _)| t == "assist/tts/pixel")
                && p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
        })
        .await;
    let published = publisher.published.lock().unwrap().clone();
    let (_, answer) = published
        .iter()
        .find(|(t, _)| t == "assist/tts/pixel")
        .unwrap()
        .clone();
    let v: serde_json::Value = serde_json::from_str(&answer).unwrap();
    let text = v["text"].as_str().unwrap();
    // The smoke skill's `speak_time_fr` always opens with "il est …"
    // (minuit/midi/"{h} heures"), regardless of the current wall-clock hour.
    assert!(text.contains("il est"), "unexpected answer: {text}");

    // Loader lifecycle: in progress before the answer, done at/after it.
    let idx = |pred: &dyn Fn(&(String, String)) -> bool| published.iter().position(|x| pred(x));
    let in_progress = idx(&|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
        .expect("in-progress status");
    let answer_idx = idx(&|(t, _)| t == "assist/tts/pixel").unwrap();
    let done =
        idx(&|(t, m)| t == "assist/llm/pixel/status" && m.contains("done")).expect("done status");
    assert!(in_progress < answer_idx && answer_idx <= done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_question_supersedes_first() {
    let publisher = RecordingPublisher::new();
    let bridge = build_bridge(publisher.clone()).await;

    // Send the same question twice, back-to-back with no delay, to exercise
    // the bridge's barge-in path (the second supersedes the first's
    // in-flight buffer/answer_deadline).
    assert!(bridge.handle(
        "assist/transcription/pixel",
        br#"{"text": "quelle heure est-il"}"#
    ));
    assert!(bridge.handle(
        "assist/transcription/pixel",
        br#"{"text": "quelle heure est-il"}"#
    ));

    // Both questions must actually be forwarded to the router: each question
    // arm publishes its own "in progress" status synchronously before the
    // transcript is sent downstream, so this is deterministic — a regression
    // that silently dropped the second question would leave this at 1.
    publisher
        .wait_for(|p| {
            p.iter()
                .filter(|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
                .count()
                >= 2
        })
        .await;
    let in_progress_count = publisher
        .published
        .lock()
        .unwrap()
        .iter()
        .filter(|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
        .count();
    assert_eq!(
        in_progress_count, 2,
        "both questions must reach the router and open their own in-progress status"
    );

    // Wait for the settled answer AND its done status before starting the
    // quiescence window below — otherwise an answer landing between the two
    // snapshots (e.g. Q2's WASM dispatch slipping past the first 1 s sleep)
    // would spuriously fail the stability check. Only one tts answer is ever
    // expected here: the router's barge-in reliably cancels Q1's dispatch
    // outcome (epoch mismatch) before it can reach `tts_tok_tx`, confirmed
    // empirically stable across repeated runs — so gating on `done` (which
    // always follows the one real answer) is the deterministic signal that
    // both questions have fully settled.
    publisher
        .wait_for(|p| {
            p.iter().any(|(t, _)| t == "assist/tts/pixel")
                && p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
        })
        .await;

    // Let anything still in flight settle, then assert quiescence: no
    // further publishes and at least one `done` status recorded.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let settled = publisher.published.lock().unwrap().len();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let still = publisher.published.lock().unwrap().len();
    assert_eq!(
        settled, still,
        "publish count must stabilize once both questions have settled"
    );

    let published = publisher.published.lock().unwrap().clone();
    assert!(
        published
            .iter()
            .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done")),
        "expected at least one done status: {published:?}"
    );
}
