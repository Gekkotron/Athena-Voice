use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use athena_voice_core::event::{BargeInReason, Event};
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::types::Transcript;
use athena_voice_skill_sdk::SkillResponse;

use crate::intent::{IntentMatcher, RuleIndex};
use crate::wasm::dispatcher::SkillDispatcherHandle;

/// Dependencies handed to the router at spawn time.
pub struct RouterDeps {
    pub llm_tx: mpsc::Sender<String>,
    /// Direct-to-TTS token channel. When a skill returns `SkillResponse::Speak`,
    /// the router feeds the speech text here, bypassing the LLM entirely.
    pub tts_tok_tx: mpsc::Sender<String>,
    pub event_tx: broadcast::Sender<Event>,
    pub session: SessionId,
    pub locale: Locale,
    /// The pattern matcher — always present now that Plan 4 Task 7 shipped.
    pub matcher: Arc<IntentMatcher>,
    /// Rule index aggregated from loaded skills. Empty if no skills are configured.
    pub rules: Arc<RuleIndex>,
    /// When present, matched intents dispatch through this handle and skip the
    /// LLM fallback path. When absent, matched intents still emit
    /// `Event::SkillInvoked` and fall through to LLM (legacy behaviour used by
    /// bootstraps that haven't loaded any skills yet).
    pub dispatcher: Option<SkillDispatcherHandle>,
}

/// Result of a spawned dispatch, forwarded back to the router main loop for
/// epoch validation and forwarding to TTS/LLM.
struct DispatchOutcome {
    epoch: u64,
    skill: String,
    result: Result<SkillResponse, athena_voice_skill_sdk::SkillError>,
}

/// Handle to a still-in-flight dispatch. The router cancels the token when a
/// newer final transcript supersedes this one; the underlying blocking WASM
/// call finishes naturally and its result is dropped.
struct PendingDispatch {
    epoch: u64,
    skill: String,
    cancel: CancellationToken,
}

/// Router: on each final transcript, run the pattern matcher; if it matches an
/// intent, emit `Event::IntentMatched` and — when a `SkillDispatcher` is wired —
/// dispatch to the owning skill, forwarding the response speech to TTS and
/// skipping the LLM fallback. If no match (or no dispatcher), fall back to LLM.
///
/// Barge-in: the router keeps a per-session `utterance_epoch`, bumped on every
/// final transcript. When a new final arrives while a prior utterance's
/// dispatch or forwarded TTS is still in flight, the router emits
/// `Event::BargeIn { reason: NewFinalTranscript }` and cancels the awaiting
/// side of the pending dispatch. Any late-arriving dispatch response whose
/// epoch has moved on is dropped and reported via `Event::SkillCancelled`.
pub fn spawn_router(
    mut rx: mpsc::Receiver<Transcript>,
    deps: RouterDeps,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut epoch: u64 = 0;
        // Currently-cancellable dispatch, if any.
        let mut pending_dispatch: Option<PendingDispatch> = None;
        // True when we've already forwarded speech/prompt to TTS or LLM for
        // the current epoch — used to detect "prior utterance still in flight"
        // once its dispatch has resolved. Cleared when a newer final transcript
        // fires barge-in (the downstream stages flush on the event).
        let mut prior_work_in_flight: bool = false;
        let (outcome_tx, mut outcome_rx) = mpsc::channel::<DispatchOutcome>(16);
        let mut rx_closed = false;

        loop {
            // Exit only once the transcript stream is drained AND no dispatch
            // is still in flight — otherwise the outcome would be lost.
            if rx_closed && pending_dispatch.is_none() {
                break;
            }
            tokio::select! {
                () = cancel.cancelled() => break,
                Some(outcome) = outcome_rx.recv() => {
                    handle_outcome(&deps, outcome, epoch, &mut pending_dispatch, &mut prior_work_in_flight).await;
                }
                maybe = rx.recv(), if !rx_closed => match maybe {
                    Some(t) if t.is_final => {
                        epoch = epoch.saturating_add(1);
                        // Detect barge-in: anything from the previous utterance
                        // still outstanding? A pending dispatch counts; so does
                        // speech we've already forwarded to TTS/LLM.
                        let has_prior_work = pending_dispatch.is_some() || prior_work_in_flight;
                        if has_prior_work {
                            let _ = deps.event_tx.send(Event::BargeIn {
                                session: deps.session,
                                reason: BargeInReason::NewFinalTranscript,
                            });
                            if let Some(p) = pending_dispatch.take() {
                                p.cancel.cancel();
                            }
                            prior_work_in_flight = false;
                        }

                        let matched = deps
                            .matcher
                            .find_match(&t.text, deps.locale.as_str(), &deps.rules);
                        if let Some(m) = matched {
                            let core_intent = athena_voice_core::types::Intent {
                                name: m.intent.name.clone(),
                                slots: m.intent.slots.clone(),
                                confidence: m.confidence,
                            };
                            let _ = deps.event_tx.send(Event::IntentMatched {
                                session: deps.session,
                                intent: core_intent,
                            });
                            if let Some(dispatcher) = &deps.dispatcher {
                                debug!(
                                    skill = %m.skill,
                                    intent = %m.intent.name,
                                    confidence = m.confidence,
                                    "dispatching matched intent to skill"
                                );
                                let dispatcher = dispatcher.clone();
                                let outcome_tx = outcome_tx.clone();
                                let dispatch_cancel = CancellationToken::new();
                                let dispatch_cancel_child = dispatch_cancel.clone();
                                let session = deps.session;
                                let skill = m.skill.clone();
                                let intent = m.intent.clone();
                                let this_epoch = epoch;
                                let skill_for_task = skill.clone();
                                tokio::spawn(async move {
                                    let result = tokio::select! {
                                        r = dispatcher.call(session, skill_for_task.clone(), intent) => Some(r),
                                        () = dispatch_cancel_child.cancelled() => None,
                                    };
                                    // On cancel the blocking WASM task keeps
                                    // running to completion and its result is
                                    // dropped. Report cancellation via a stub
                                    // outcome so the router emits the observable.
                                    let result = result.unwrap_or_else(|| Err(
                                        athena_voice_skill_sdk::SkillError::Custom(
                                            "dispatch cancelled by barge-in".into(),
                                        ),
                                    ));
                                    let _ = outcome_tx
                                        .send(DispatchOutcome {
                                            epoch: this_epoch,
                                            skill: skill_for_task,
                                            result,
                                        })
                                        .await;
                                });
                                pending_dispatch = Some(PendingDispatch {
                                    epoch,
                                    skill,
                                    cancel: dispatch_cancel,
                                });
                                continue;
                            }
                            // No dispatcher wired: keep the legacy observable of
                            // emitting SkillInvoked and falling through to LLM
                            // so the router remains useful before skills load.
                            let _ = deps.event_tx.send(Event::SkillInvoked {
                                session: deps.session,
                                skill: m.skill.clone(),
                            });
                        }
                        let _ = deps.event_tx.send(Event::LlmFallback { session: deps.session });
                        if deps.llm_tx.send(t.text).await.is_err() {
                            break;
                        }
                        prior_work_in_flight = true;
                    }
                    Some(_) => {} // partials dropped
                    None => { rx_closed = true; }
                }
            }
        }
    })
}

async fn handle_outcome(
    deps: &RouterDeps,
    outcome: DispatchOutcome,
    current_epoch: u64,
    pending_dispatch: &mut Option<PendingDispatch>,
    prior_work_in_flight: &mut bool,
) {
    let DispatchOutcome {
        epoch,
        skill,
        result,
    } = outcome;

    // Clear the pending slot if this outcome corresponds to it.
    if pending_dispatch
        .as_ref()
        .is_some_and(|p| p.epoch == epoch && p.skill == skill)
    {
        *pending_dispatch = None;
    }

    if epoch != current_epoch {
        // Superseded by a newer utterance.
        let _ = deps.event_tx.send(Event::SkillCancelled {
            session: deps.session,
            skill,
        });
        return;
    }

    match result {
        Ok(SkillResponse::Speak { text }) => {
            let text = ensure_sentence_boundary(text);
            if deps.tts_tok_tx.send(text).await.is_err() {
                return;
            }
            *prior_work_in_flight = true;
        }
        Ok(SkillResponse::AskLlm { prompt }) => {
            let _ = deps.event_tx.send(Event::LlmFallback {
                session: deps.session,
            });
            if deps.llm_tx.send(prompt).await.is_err() {
                return;
            }
            *prior_work_in_flight = true;
        }
        Ok(SkillResponse::Empty) => {}
        Err(err) => {
            warn!(skill = %skill, error = %err, "skill dispatch failed");
        }
    }
}

fn ensure_sentence_boundary(mut text: String) -> String {
    if !matches!(text.chars().last(), Some('.' | '!' | '?')) {
        text.push('.');
    }
    text
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use athena_voice_skill_sdk::{Intent as SkillIntent, PatternRule, SkillError};

    use super::*;
    use crate::intent::rule::HostPatternRule;
    use crate::wasm::dispatcher::SkillDispatcher;
    use crate::wasm::registry::{SkillPlugin, SkillRegistry};

    fn build_deps(
        llm_tx: mpsc::Sender<String>,
        event_tx: broadcast::Sender<Event>,
        rules: Arc<RuleIndex>,
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
            dispatcher: None,
        }
    }

    #[tokio::test]
    async fn empty_rules_falls_back_to_llm() {
        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, mut llm_rx) = mpsc::channel(4);
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let deps = build_deps(llm_tx, ev_tx, Arc::new(RuleIndex::new()));

        let handle = spawn_router(t_rx, deps, CancellationToken::new());

        t_tx.send(Transcript {
            text: "quelle heure est-il".into(),
            is_final: true,
            confidence: None,
        })
        .await
        .unwrap();
        drop(t_tx);

        assert_eq!(llm_rx.recv().await.unwrap(), "quelle heure est-il");
        handle.await.unwrap();

        let mut got_fallback = false;
        while let Ok(ev) = ev_rx.try_recv() {
            if matches!(ev, Event::LlmFallback { .. }) {
                got_fallback = true;
            }
        }
        assert!(got_fallback);
    }

    #[tokio::test]
    async fn matched_intent_emits_events_but_still_falls_back_to_llm() {
        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, mut llm_rx) = mpsc::channel(4);
        let (ev_tx, mut ev_rx) = broadcast::channel(32);

        // Load one rule.
        let mut index = RuleIndex::new();
        index.insert(
            "fr".into(),
            HostPatternRule::from(PatternRule {
                intent: "time.query".into(),
                phrases: vec!["quelle heure est-il".into()],
                slots: Vec::new(),
            }),
            "clock".into(),
        );
        let deps = build_deps(llm_tx, ev_tx, Arc::new(index));

        let handle = spawn_router(t_rx, deps, CancellationToken::new());

        t_tx.send(Transcript {
            text: "quelle heure est-il".into(),
            is_final: true,
            confidence: None,
        })
        .await
        .unwrap();
        drop(t_tx);

        // LLM fallback still fires until dispatcher exists.
        assert_eq!(llm_rx.recv().await.unwrap(), "quelle heure est-il");
        handle.await.unwrap();

        let mut got_matched = false;
        let mut got_skill_invoked = false;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Event::IntentMatched { intent, .. } => {
                    assert_eq!(intent.name, "time.query");
                    got_matched = true;
                }
                Event::SkillInvoked { skill, .. } => {
                    assert_eq!(skill, "clock");
                    got_skill_invoked = true;
                }
                _ => {}
            }
        }
        assert!(got_matched, "expected IntentMatched");
        assert!(got_skill_invoked, "expected SkillInvoked");
    }

    #[tokio::test]
    async fn partials_are_dropped() {
        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, mut llm_rx) = mpsc::channel(4);
        let (ev_tx, _ev_rx) = broadcast::channel(16);
        let deps = build_deps(llm_tx, ev_tx, Arc::new(RuleIndex::new()));

        spawn_router(t_rx, deps, CancellationToken::new());

        t_tx.send(Transcript {
            text: "bon".into(),
            is_final: false,
            confidence: None,
        })
        .await
        .unwrap();
        drop(t_tx);
        assert!(llm_rx.recv().await.is_none());
    }

    // --- Barge-in tests -----------------------------------------------------

    /// A mock plugin whose `handle` runs the supplied closure on a
    /// spawn-blocking thread — the closure decides how long to stall the
    /// dispatch, letting the test reproduce the "second final arrives while
    /// first is still in flight" race deterministically.
    struct StallPlugin<F>
    where
        F: FnMut(&SkillIntent) -> Result<SkillResponse, SkillError> + Send,
    {
        handle: F,
    }

    impl<F> SkillPlugin for StallPlugin<F>
    where
        F: FnMut(&SkillIntent) -> Result<SkillResponse, SkillError> + Send,
    {
        fn pattern_rules(&mut self, _locale: &str) -> Result<Vec<PatternRule>, extism::Error> {
            Ok(Vec::new())
        }
        fn handle(&mut self, intent: &SkillIntent) -> Result<SkillResponse, SkillError> {
            (self.handle)(intent)
        }
    }

    fn install_clock_rule(index: &mut RuleIndex, skill: &str, phrase: &str) {
        index.insert(
            "fr".into(),
            HostPatternRule::from(PatternRule {
                intent: "time.query".into(),
                phrases: vec![phrase.into()],
                slots: Vec::new(),
            }),
            skill.into(),
        );
    }

    fn build_deps_with_dispatcher(
        llm_tx: mpsc::Sender<String>,
        tts_tok_tx: mpsc::Sender<String>,
        event_tx: broadcast::Sender<Event>,
        rules: Arc<RuleIndex>,
        dispatcher: SkillDispatcherHandle,
    ) -> RouterDeps {
        RouterDeps {
            llm_tx,
            tts_tok_tx,
            event_tx,
            session: SessionId::new_v4(),
            locale: Locale::new("fr").unwrap(),
            matcher: Arc::new(IntentMatcher::new()),
            rules,
            dispatcher: Some(dispatcher),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn barge_in_drops_superseded_speech_and_emits_events() {
        let mut index = RuleIndex::new();
        install_clock_rule(&mut index, "clock", "quelle heure est-il");
        install_clock_rule(&mut index, "clock", "il est quelle heure");

        // Skill that stalls the first call long enough for the second
        // transcript to supersede it. We gate the first call on a channel
        // that never closes until the test signals it — but for determinism
        // we simply sleep long enough. The second call returns immediately.
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let call_count_clone = call_count.clone();
        let plugin: Arc<Mutex<dyn SkillPlugin>> = Arc::new(Mutex::new(StallPlugin {
            handle: move |_intent: &SkillIntent| {
                let n = call_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    // First call: stall long enough for the router to see
                    // barge-in and cancel our await.
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    Ok(SkillResponse::speak("réponse tardive"))
                } else {
                    Ok(SkillResponse::speak("il est huit heures"))
                }
            },
        }));
        let mut reg = SkillRegistry::new();
        reg.install("clock", plugin, &[]).unwrap();
        let reg = Arc::new(reg);

        let (ev_tx, mut ev_rx) = broadcast::channel(64);
        let (dispatcher, dispatcher_task) =
            SkillDispatcher::spawn(reg, ev_tx.clone(), CancellationToken::new());

        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, _llm_rx) = mpsc::channel(4);
        let (tts_tok_tx, mut tts_tok_rx) = mpsc::channel(4);

        let deps = build_deps_with_dispatcher(
            llm_tx,
            tts_tok_tx,
            ev_tx.clone(),
            Arc::new(index),
            dispatcher.clone(),
        );

        let router_cancel = CancellationToken::new();
        let router_handle = spawn_router(t_rx, deps, router_cancel.clone());

        // First final: schedules a stalled dispatch.
        t_tx.send(Transcript {
            text: "quelle heure est-il".into(),
            is_final: true,
            confidence: None,
        })
        .await
        .unwrap();

        // Give the router a moment to spawn the dispatch task and register it.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Second final: should trigger barge-in + cancel the first dispatch.
        t_tx.send(Transcript {
            text: "il est quelle heure".into(),
            is_final: true,
            confidence: None,
        })
        .await
        .unwrap();

        // Only the second dispatch's speech should reach TTS.
        let tok = tokio::time::timeout(std::time::Duration::from_secs(2), tts_tok_rx.recv())
            .await
            .expect("timed out waiting for TTS token")
            .expect("tts_tok_tx closed unexpectedly");
        assert_eq!(tok, "il est huit heures.");

        // Assert no further token arrives from the stalled first dispatch —
        // wait long enough for its 400ms stall to have completed.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(
            tts_tok_rx.try_recv().is_err(),
            "superseded speech leaked to TTS"
        );

        drop(t_tx);
        router_cancel.cancel();
        let _ = router_handle.await;
        drop(dispatcher);
        let _ = dispatcher_task.await;

        let mut saw_barge_in = false;
        let mut saw_skill_cancelled = false;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Event::BargeIn {
                    reason: BargeInReason::NewFinalTranscript,
                    ..
                } => saw_barge_in = true,
                Event::SkillCancelled { skill, .. } => {
                    assert_eq!(skill, "clock");
                    saw_skill_cancelled = true;
                }
                _ => {}
            }
        }
        assert!(saw_barge_in, "expected Event::BargeIn");
        assert!(saw_skill_cancelled, "expected Event::SkillCancelled");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn single_final_transcript_does_not_emit_barge_in() {
        let mut index = RuleIndex::new();
        install_clock_rule(&mut index, "clock", "quelle heure est-il");
        let plugin: Arc<Mutex<dyn SkillPlugin>> = Arc::new(Mutex::new(StallPlugin {
            handle: |_intent: &SkillIntent| Ok(SkillResponse::speak("il est huit heures")),
        }));
        let mut reg = SkillRegistry::new();
        reg.install("clock", plugin, &[]).unwrap();
        let reg = Arc::new(reg);

        let (ev_tx, mut ev_rx) = broadcast::channel(64);
        let (dispatcher, dispatcher_task) =
            SkillDispatcher::spawn(reg, ev_tx.clone(), CancellationToken::new());

        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, _llm_rx) = mpsc::channel(4);
        let (tts_tok_tx, mut tts_tok_rx) = mpsc::channel(4);

        let deps = build_deps_with_dispatcher(
            llm_tx,
            tts_tok_tx,
            ev_tx.clone(),
            Arc::new(index),
            dispatcher.clone(),
        );

        let router_cancel = CancellationToken::new();
        let router_handle = spawn_router(t_rx, deps, router_cancel.clone());

        t_tx.send(Transcript {
            text: "quelle heure est-il".into(),
            is_final: true,
            confidence: None,
        })
        .await
        .unwrap();

        let tok = tokio::time::timeout(std::time::Duration::from_secs(2), tts_tok_rx.recv())
            .await
            .expect("timed out")
            .expect("tts_tok_tx closed");
        assert_eq!(tok, "il est huit heures.");

        drop(t_tx);
        router_cancel.cancel();
        let _ = router_handle.await;
        drop(dispatcher);
        let _ = dispatcher_task.await;

        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Event::BargeIn { .. } => panic!("unexpected BargeIn on single utterance"),
                Event::SkillCancelled { .. } => {
                    panic!("unexpected SkillCancelled on single utterance")
                }
                _ => {}
            }
        }
    }
}
