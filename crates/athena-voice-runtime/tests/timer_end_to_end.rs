//! End-to-end timer skill dispatch (Task C).
//!
//! Loads the `skills-timer` WASM built by `build.rs` (path exposed via the
//! `TIMER_TEST_WASM` env var), wires it into a `SkillRegistry` +
//! `SkillDispatcher`, spawns the runtime's router, TTS actor, `SchedulerTask`,
//! and the `SkillNotify` forwarder, feeds a final French transcript, and
//! asserts:
//!
//! - `Event::IntentMatched` fires with `timer.set`.
//! - `Event::SkillInvoked` fires (emitted by the dispatcher).
//! - The TTS pipeline emits a chunk carrying the "d'accord, minuteur" speech.
//! - After the scheduled duration elapses, `Event::ScheduledFired` fires and
//!   a follow-up TTS chunk carries the expiration announce.
//!
//! Note: the scheduler reads real wall-clock time (`chrono::Utc::now()`) both
//! when the guest computes `fires_at_ms` (via `SystemTime::now()` inside the
//! WASI sandbox) and when the host scheduler tick checks `pop_due_events`.
//! Tokio's paused-clock test utilities only mock tokio's internal timer, not
//! the system clock, so this test uses a real (short) sleep rather than
//! `tokio::time::pause`/`advance` to let the two-second timer actually elapse.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
use athena_voice_runtime::mqtt::{MqttClient, MqttConfig};
use athena_voice_runtime::pipeline::router::{RouterDeps, spawn_router};
use athena_voice_runtime::pipeline::tts::spawn_tts;
use athena_voice_runtime::wasm::dispatcher::SkillDispatcher;
use athena_voice_runtime::wasm::host_fns::{AsyncClientPublisher, SkillCtx, host_functions};
use athena_voice_runtime::wasm::registry::{ExtismSkillPlugin, SkillPlugin, SkillRegistry};
use athena_voice_runtime::wasm::scheduler::{SchedulerTask, spawn_skill_notify_forwarder};
use athena_voice_storage::{SqliteStore, Store};

const SKILL_NAME: &str = "timer";

#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timer_set_then_expires() {
    let _ = tracing_subscriber::fmt::try_init();
    let wasm_path = PathBuf::from(env!("TIMER_TEST_WASM"));
    assert!(
        wasm_path.exists(),
        "timer wasm missing at {}: build.rs should have produced it",
        wasm_path.display()
    );

    // ---------- Skill runtime deps ----------
    let store: Arc<dyn Store> = Arc::new(
        SqliteStore::open("sqlite::memory:")
            .await
            .expect("sqlite in-memory store"),
    );
    // Bogus broker: the client never actually connects, but publishes just
    // enqueue in-memory (queue depth 128).
    let mqtt = MqttClient::connect(MqttConfig {
        host: "127.0.0.1".into(),
        port: 62992,
        client_id: "athena-voice-timer-test".into(),
        username: None,
        password: None,
        keep_alive_secs: 30,
    })
    .expect("mqtt client");
    let http = reqwest::Client::new();

    let ctx = SkillCtx {
        name: SKILL_NAME.into(),
        store: store.clone(),
        mqtt: Arc::new(AsyncClientPublisher(mqtt.tx.clone())),
        http_allowlist: Vec::new(),
        mqtt_publish_allowlist: Vec::new(),
        config: HashMap::new(),
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
        .expect("install timer skill");
    let rules = registry.patterns_handle();
    let registry = Arc::new(registry);

    // ---------- Actor DAG ----------
    let (ev_tx, mut ev_rx) = broadcast::channel::<Event>(128);
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

    let (llm_tx, mut llm_rx) = mpsc::channel::<String>(4);
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

    let scheduler_task = SchedulerTask::spawn(
        store.clone(),
        mqtt.tx.clone(),
        ev_tx.clone(),
        session,
        cancel.clone(),
    );
    let forwarder_task =
        spawn_skill_notify_forwarder(ev_tx.subscribe(), tok_tx.clone(), cancel.clone());

    // ---------- Drive the pipeline: set a two-second timer ----------
    t_tx.send(Transcript {
        text: "mets un minuteur de deux secondes".into(),
        is_final: true,
        confidence: None,
    })
    .await
    .expect("send transcript");

    let first_tok = timeout(Duration::from_secs(10), chunk_rx.recv())
        .await
        .expect("timed out waiting for first TTS chunk")
        .expect("expected a TTS chunk");
    assert!(
        String::from_utf8_lossy(&first_tok).contains("d'accord")
            || String::from_utf8_lossy(&first_tok).contains("minuteur"),
        "expected the 'd'accord, minuteur' speech; got {first_tok:?}"
    );

    // Let the two-second timer actually elapse in real wall-clock time, plus
    // slack for the 1s scheduler tick cadence.
    tokio::time::sleep(Duration::from_secs(4)).await;

    // A follow-up TTS chunk should carry the expiration announce. `"terminé"`
    // (from `"le minuteur de N secondes est terminé"`) only appears in the
    // expiration text, not in the initial "d'accord, minuteur ... lancé"
    // confirmation, so it disambiguates from leftover word-chunks of the
    // first response still queued in `chunk_rx`.
    let mut saw_expiration_chunk = false;
    while let Ok(Some(chunk)) = timeout(Duration::from_secs(3), chunk_rx.recv()).await {
        if String::from_utf8_lossy(&chunk).contains("terminé") {
            saw_expiration_chunk = true;
        }
    }
    assert!(
        saw_expiration_chunk,
        "expected a follow-up TTS chunk carrying the expiration announce"
    );
    assert!(
        llm_rx.try_recv().is_err(),
        "LLM must not receive a prompt when a skill handled the intent"
    );

    // ---------- Shutdown ----------
    drop(t_tx);
    drop(tok_tx);
    cancel.cancel();
    let _ = router_task.await;
    let _ = tts_task.await;
    let _ = scheduler_task.await;
    let _ = forwarder_task.await;
    drop(dispatcher_handle);
    let _ = dispatcher_task.await;

    // ---------- Assert on emitted events ----------
    let mut saw_intent_matched = false;
    let mut saw_skill_invoked = false;
    let mut saw_scheduled_fired = false;
    loop {
        match ev_rx.try_recv() {
            Ok(Event::IntentMatched { intent, .. }) => {
                assert_eq!(intent.name, "timer.set");
                saw_intent_matched = true;
            }
            Ok(Event::SkillInvoked { skill, .. }) => {
                assert_eq!(skill, SKILL_NAME);
                saw_skill_invoked = true;
            }
            Ok(Event::ScheduledFired { skill, .. }) => {
                assert_eq!(skill, SKILL_NAME);
                saw_scheduled_fired = true;
            }
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
    assert!(saw_intent_matched, "expected Event::IntentMatched");
    assert!(saw_skill_invoked, "expected Event::SkillInvoked");
    assert!(saw_scheduled_fired, "expected Event::ScheduledFired");

    // Confirm the store no longer has the scheduled event (consumed).
    let due = store.pop_due_events(i64::MAX).await.unwrap();
    assert!(due.is_empty(), "scheduled event should have been consumed");
}
