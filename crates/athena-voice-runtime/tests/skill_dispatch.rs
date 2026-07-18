//! End-to-end skill dispatch (Plan 4 Task 11).
//!
//! Loads the `skills-smoke-test` WASM built by `build.rs` (path exposed via
//! the `SMOKE_TEST_WASM` env var), wires it into a `SkillRegistry` +
//! `SkillDispatcher`, spawns the runtime's router and TTS actors, feeds a
//! final French transcript, and asserts:
//!
//! - `Event::IntentMatched` fires with `time.query`.
//! - `Event::SkillInvoked` fires (emitted by the dispatcher).
//! - The TTS pipeline emits chunks carrying the skill's response speech.
//! - `Event::LlmFallback` never fires — the router bypasses the LLM when a
//!   skill is dispatched.
//!
//! MQTT roundtrip through `Runtime::spawn` is deliberately avoided: it needs
//! a broker and is orthogonal to the skill-dispatch contract this test pins.

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
use athena_voice_runtime::wasm::host_fns::{SkillCtx, host_functions};
use athena_voice_runtime::wasm::registry::{ExtismSkillPlugin, SkillPlugin, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};

/// The skill name must match the ACL prefix baked into the smoke skill
/// (`athena/skills/smoke-test/…`) so `mqtt_publish` succeeds; the guest ABI
/// derives ACL from `SkillCtx.name`, not the wasm file stem.
const SKILL_NAME: &str = "smoke-test";

#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_to_end_skill_dispatch() {
    let _ = tracing_subscriber::fmt::try_init();
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
        port: 62991,
        client_id: "athena-voice-skill-test".into(),
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
        mqtt: mqtt.tx.clone(),
        // Allowlist matches the URL the smoke skill hits; the call itself
        // fails at DNS (smoke.local doesn't resolve), but the skill discards
        // the result with `let _ = ...`, so ACL passes and the plugin
        // completes normally.
        http_allowlist: vec!["smoke.local".into()],
        config,
        tokio: tokio::runtime::Handle::current(),
        http,
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

    // ---------- Drive the pipeline ----------
    t_tx.send(Transcript {
        text: "quelle heure est-il".into(),
        is_final: true,
        confidence: None,
    })
    .await
    .expect("send transcript");
    drop(t_tx);
    // Router keeps its own tts_tok_tx clone; drop ours so TTS can eventually
    // flush + shut down once the router exits.
    drop(tok_tx);

    // Collect TTS chunks until the channel closes.
    let mut chunks: Vec<Bytes> = Vec::new();
    while let Ok(Some(chunk)) = timeout(Duration::from_secs(10), chunk_rx.recv()).await {
        chunks.push(chunk);
    }

    assert!(!chunks.is_empty(), "expected at least one TTS chunk");
    // FakeTts emits one chunk per whitespace-separated word. Concatenate the
    // words and confirm the skill's expected speech survived the trip.
    let joined = chunks
        .iter()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        joined.contains("huit") && joined.contains("heure"),
        "expected TTS chunks to carry the skill speech; joined chunks: {joined:?}"
    );

    assert!(
        llm_rx.try_recv().is_err(),
        "LLM must not receive a prompt when a skill handled the intent"
    );

    // ---------- Shutdown ----------
    cancel.cancel();
    let _ = router_task.await;
    let _ = tts_task.await;
    drop(dispatcher_handle);
    let _ = dispatcher_task.await;

    // ---------- Assert on emitted events ----------
    let mut saw_intent_matched = false;
    let mut saw_skill_invoked = false;
    let mut saw_llm_fallback = false;
    let mut saw_tts_chunk = false;
    loop {
        match ev_rx.try_recv() {
            Ok(Event::IntentMatched { intent, .. }) => {
                assert_eq!(intent.name, "time.query");
                saw_intent_matched = true;
            }
            Ok(Event::SkillInvoked { skill, .. }) => {
                assert_eq!(skill, SKILL_NAME);
                saw_skill_invoked = true;
            }
            Ok(Event::LlmFallback { .. }) => saw_llm_fallback = true,
            Ok(Event::TtsChunk { .. }) => saw_tts_chunk = true,
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
    assert!(saw_intent_matched, "expected Event::IntentMatched");
    assert!(saw_skill_invoked, "expected Event::SkillInvoked");
    assert!(saw_tts_chunk, "expected at least one Event::TtsChunk");
    assert!(
        !saw_llm_fallback,
        "Event::LlmFallback must NOT fire when a skill handles the intent"
    );
}
