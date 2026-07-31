#![allow(unused_imports)]
use std::sync::Arc;

use arc_swap::ArcSwap;
use athena_voice_core::event::Event;
use athena_voice_core::event::LlmFallbackReason as FallbackReason;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::types::Transcript;
use athena_voice_runtime::intent::engine::IntentMatcher;
use athena_voice_runtime::intent::loader::RuleIndex;
use athena_voice_runtime::intent::rule::{HostPatternRule, HostSlotKind, HostSlotSpec};
use athena_voice_runtime::pipeline::router::{RouterDeps, spawn_router};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn missing_slot_triggers_llm_fallback() {
    let (t_tx, t_rx) = mpsc::channel(4);
    let (llm_tx, mut llm_rx) = mpsc::channel(4);
    let (ev_tx, mut ev_rx) = broadcast::channel(16);

    // Load a rule with a slot
    let mut index = RuleIndex::new();
    let rule = HostPatternRule {
        intent: "weather.query".into(),
        phrases: vec!["météo à {city}".into()],
        slots: vec![HostSlotSpec {
            name: "city".into(),
            kind: HostSlotKind::String,
        }],
    };
    index.insert("fr".into(), rule, "weather".into());

    let deps = build_deps(llm_tx, ev_tx, Arc::new(ArcSwap::from_pointee(index)));

    let router_cancel = CancellationToken::new();
    let router_handle = spawn_router(t_rx, deps, router_cancel.clone());

    // Send a transcript missing the slot value
    t_tx.send(Transcript {
        text: "météo à".into(), // Missing city
        is_final: true,
        confidence: None,
    })
    .await
    .unwrap();
    drop(t_tx);

    // Expect LLM fallback for missing slot
    let prompt = llm_rx.recv().await.unwrap();
    assert!(
        prompt.contains("needs the value for slots city"),
        "Prompt should ask for missing slot: {prompt}",
    );

    // Check emitted events
    let mut got_intent_matched = false;
    let mut got_llm_fallback = false;
    while let Ok(ev) = ev_rx.try_recv() {
        match ev {
            Event::IntentMatched { intent, .. } => {
                assert_eq!(intent.name, "weather.query");
                assert_eq!(intent.slots["city"], json!(null));
                got_intent_matched = true;
            }
            Event::LlmFallback { reason, slots, .. } => {
                assert!(matches!(reason, FallbackReason::MissingSlots));
                assert_eq!(slots, vec!["city"]);
                got_llm_fallback = true;
            }
            _ => {}
        }
    }
    assert!(got_intent_matched, "Expected Event::IntentMatched");
    assert!(
        got_llm_fallback,
        "Expected Event::LlmFallback for missing slot"
    );

    router_cancel.cancel();
    router_handle.await.unwrap();
}

fn build_deps(
    llm_tx: mpsc::Sender<String>,
    event_tx: broadcast::Sender<Event>,
    rules: Arc<ArcSwap<RuleIndex>>,
) -> RouterDeps {
    let (tts_tok_tx, _tts_tok_rx) = mpsc::channel(4);
    RouterDeps {
        llm_tx,
        tts_tok_tx,
        event_tx,
        session: SessionId::new_v4(),
        locale: Locale::new("fr").unwrap(),
        matcher: Arc::new(IntentMatcher::new()),
        rules,
        dispatcher: None, // Test only the router logic
    }
}
