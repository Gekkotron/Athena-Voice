use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::types::Transcript;

use crate::intent::{IntentMatcher, RuleIndex};

/// Dependencies handed to the router at spawn time.
pub struct RouterDeps {
    pub llm_tx: mpsc::Sender<String>,
    pub event_tx: broadcast::Sender<Event>,
    pub session: SessionId,
    pub locale: Locale,
    /// The pattern matcher — always present now that Plan 4 Task 7 shipped.
    pub matcher: Arc<IntentMatcher>,
    /// Rule index aggregated from loaded skills. Empty if no skills are configured.
    pub rules: Arc<RuleIndex>,
}

/// Router: on each final transcript, run the pattern matcher; if it matches an
/// intent, emit `Event::IntentMatched` (skill dispatch will land with Plan 4
/// Task 9's completion — for now, matched intents still fall through to LLM
/// because there's no `SkillDispatcher` yet). If no match, always fall back to
/// LLM as before.
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
                            let _ = deps.event_tx.send(Event::SkillInvoked {
                                session: deps.session,
                                skill: m.skill.clone(),
                            });
                            debug!(
                                skill = %m.skill,
                                intent = %m.intent.name,
                                confidence = m.confidence,
                                "matched intent (skill dispatch not yet wired; falling through to LLM)"
                            );
                            // TODO(Plan 4 Task 6+): dispatch to SkillRegistry here
                            // instead of falling through.
                        }
                        // Fall through to LLM whether or not we matched (Plan 4 Task 9 will
                        // replace this branch once SkillDispatcher exists).
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
        RouterDeps {
            llm_tx,
            event_tx,
            session: SessionId::new_v4(),
            locale: Locale::new("fr").unwrap(),
            matcher: Arc::new(IntentMatcher::new()),
            rules,
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
