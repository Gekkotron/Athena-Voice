use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use athena_voice_core::event::Event;
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

/// Router: on each final transcript, run the pattern matcher; if it matches an
/// intent, emit `Event::IntentMatched` and — when a `SkillDispatcher` is wired —
/// dispatch to the owning skill, forwarding the response speech to TTS and
/// skipping the LLM fallback. If no match (or no dispatcher), fall back to LLM.
pub fn spawn_router(
    mut rx: mpsc::Receiver<Transcript>,
    deps: RouterDeps,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(t) if t.is_final => {
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
                                match dispatcher
                                    .call(deps.session, m.skill.clone(), m.intent.clone())
                                    .await
                                {
                                    Ok(SkillResponse::Speak { text }) => {
                                        let text = ensure_sentence_boundary(text);
                                        if deps.tts_tok_tx.send(text).await.is_err() {
                                            break;
                                        }
                                    }
                                    Ok(SkillResponse::AskLlm { prompt }) => {
                                        let _ = deps
                                            .event_tx
                                            .send(Event::LlmFallback { session: deps.session });
                                        if deps.llm_tx.send(prompt).await.is_err() {
                                            break;
                                        }
                                    }
                                    Ok(SkillResponse::Empty) => {}
                                    Err(err) => {
                                        warn!(skill = %m.skill, error = %err, "skill dispatch failed");
                                    }
                                }
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
                    }
                    Some(_) => {} // partials dropped
                    None => break,
                }
            }
        }
    })
}

fn ensure_sentence_boundary(mut text: String) -> String {
    if !matches!(text.chars().last(), Some('.' | '!' | '?')) {
        text.push('.');
    }
    text
}

#[cfg(test)]
mod tests {
    use athena_voice_skill_sdk::PatternRule;

    use super::*;
    use crate::intent::rule::HostPatternRule;

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
}
