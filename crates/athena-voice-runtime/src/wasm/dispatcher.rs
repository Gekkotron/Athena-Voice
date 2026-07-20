//! Skill dispatcher: a tokio actor that owns a [`SkillRegistry`] and turns
//! matched `(session, skill, intent)` triples into `SkillResponse` values.
//!
//! Extism plugin invocation is CPU-bound and blocking (it enters a WASM VM),
//! so the dispatcher wraps every call in `tokio::task::spawn_blocking` to
//! keep the async runtime responsive. A plugin panic (surfacing as
//! `JoinError::is_panic`) becomes [`Event::SkillPanicked`] instead of
//! poisoning the task tree.
//!
//! Task 8 delivers the actor plus its channel; Task 9 wires the router to
//! push into it and the response back into TTS.

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use athena_voice_core::event::Event;
use athena_voice_core::ids::SessionId;
use athena_voice_skill_sdk::{Intent, SkillError, SkillResponse};

use crate::wasm::registry::SkillRegistry;

/// A single request pushed onto the dispatcher's mailbox.
///
/// `reply` is optional: fire-and-forget dispatches can omit it and rely on
/// `Event::SkillInvoked` / `Event::SkillPanicked` for observability.
pub struct DispatchRequest {
    pub session: SessionId,
    pub skill: String,
    pub intent: Intent,
    pub reply: Option<oneshot::Sender<Result<SkillResponse, SkillError>>>,
}

/// Handle to a running dispatcher. Cloneable — every clone shares the same
/// underlying mailbox.
#[derive(Clone)]
pub struct SkillDispatcherHandle {
    tx: mpsc::Sender<DispatchRequest>,
}

impl SkillDispatcherHandle {
    /// Enqueue a request. Returns `Err(req)` iff the dispatcher task has
    /// already shut down.
    pub async fn dispatch(&self, req: DispatchRequest) -> Result<(), DispatchRequest> {
        self.tx.send(req).await.map_err(|e| e.0)
    }

    /// Convenience: fire-and-forget dispatch.
    pub async fn send(&self, session: SessionId, skill: String, intent: Intent) -> Result<(), SkillError> {
        self.dispatch(DispatchRequest {
            session,
            skill,
            intent,
            reply: None,
        })
        .await
        .map_err(|_| SkillError::Custom("Dispatch failed".into()))
    }

    /// Convenience: request/response dispatch — waits until the plugin
    /// returns and yields the `SkillResponse`.
    pub async fn call(
        &self,
        session: SessionId,
        skill: String,
        intent: Intent,
    ) -> Result<SkillResponse, SkillError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.dispatch(DispatchRequest {
            session,
            skill,
            intent,
            reply: Some(reply_tx),
        })
        .await
        .map_err(|_| SkillError::Custom("skill dispatcher shut down".into()))?;
        reply_rx
            .await
            .map_err(|_| SkillError::Custom("skill dispatcher dropped reply".into()))?
    }
}

/// The dispatcher actor.
pub struct SkillDispatcher;

impl SkillDispatcher {
    /// Spawn the dispatcher actor. `registry` is shared and re-entered on
    /// every request; `event_tx` receives `SkillInvoked` / `SkillPanicked`.
    /// `cancel` shuts the loop down even if the mailbox still has senders.
    #[must_use]
    pub fn spawn(
        registry: Arc<SkillRegistry>,
        event_tx: broadcast::Sender<Event>,
        cancel: CancellationToken,
    ) -> (SkillDispatcherHandle, JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<DispatchRequest>(64);
        let task = tokio::spawn(async move {
            loop {
                let req = tokio::select! {
                    () = cancel.cancelled() => break,
                    maybe = rx.recv() => match maybe {
                        Some(r) => r,
                        None => break, // all senders dropped
                    },
                };
                Self::handle_one(&registry, &event_tx, req).await;
            }
            debug!("skill dispatcher shutting down");
        });
        (SkillDispatcherHandle { tx }, task)
    }

    async fn handle_one(
        registry: &Arc<SkillRegistry>,
        event_tx: &broadcast::Sender<Event>,
        req: DispatchRequest,
    ) {
        let DispatchRequest {
            session,
            skill,
            intent,
            reply,
        } = req;

        // Emit `SkillInvoked` before entering the plugin so observers see the
        // handoff even if the guest hangs or crashes.
        let _ = event_tx.send(Event::SkillInvoked {
            session,
            skill: skill.clone(),
        });

        let reg = registry.clone();
        let skill_for_blocking = skill.clone();
        let join =
            tokio::task::spawn_blocking(move || reg.dispatch(&skill_for_blocking, intent)).await;

        let outcome: Result<SkillResponse, SkillError> = match join {
            Ok(result) => result,
            Err(join_err) if join_err.is_panic() => {
                let reason = panic_reason(join_err.into_panic());
                warn!(session = %session, skill = %skill, reason = %reason, "skill panicked");
                let _ = event_tx.send(Event::SkillPanicked {
                    session,
                    skill: skill.clone(),
                    reason: reason.clone(),
                });
                Err(SkillError::Custom(format!("skill panicked: {reason}")))
            }
            Err(join_err) => {
                warn!(session = %session, skill = %skill, error = %join_err, "skill dispatch cancelled");
                Err(SkillError::Custom(format!(
                    "dispatch cancelled: {join_err}"
                )))
            }
        };

        if let Some(reply_tx) = reply {
            // Receiver may have dropped — that's fine, this is best-effort.
            let _ = reply_tx.send(outcome);
        }
    }
}

fn panic_reason(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use athena_voice_skill_sdk::PatternRule;

    use crate::wasm::registry::SkillPlugin;

    struct MockPlugin<F>
    where
        F: FnMut(&Intent) -> Result<SkillResponse, SkillError> + Send,
    {
        handle: F,
    }

    impl<F> SkillPlugin for MockPlugin<F>
    where
        F: FnMut(&Intent) -> Result<SkillResponse, SkillError> + Send,
    {
        fn pattern_rules(&mut self, _locale: &str) -> Result<Vec<PatternRule>, extism::Error> {
            Ok(Vec::new())
        }
        fn handle(&mut self, intent: &Intent) -> Result<SkillResponse, SkillError> {
            (self.handle)(intent)
        }
    }

    fn intent(name: &str) -> Intent {
        Intent {
            name: name.into(),
            slots: BTreeMap::new(),
        }
    }

    fn registry_with<F>(name: &str, handler: F) -> Arc<SkillRegistry>
    where
        F: FnMut(&Intent) -> Result<SkillResponse, SkillError> + Send + 'static,
    {
        let reg = SkillRegistry::new();
        let plugin: Arc<Mutex<dyn SkillPlugin>> =
            Arc::new(Mutex::new(MockPlugin { handle: handler }));
        reg.install(name, plugin, &[]).unwrap();
        Arc::new(reg)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn call_returns_skill_response_and_emits_invoked() {
        let reg = registry_with("clock", |_| Ok(SkillResponse::speak("il est huit heures")));
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let (handle, task) = SkillDispatcher::spawn(reg, ev_tx.clone(), CancellationToken::new());

        let session = SessionId::new_v4();
        let response = handle
            .call(session, "clock".into(), intent("time.query"))
            .await
            .unwrap();
        assert!(matches!(response, SkillResponse::Speak { text } if text == "il est huit heures"));

        drop(handle);
        task.await.unwrap();

        let mut saw_invoked = false;
        while let Ok(ev) = ev_rx.try_recv() {
            if let Event::SkillInvoked { skill, .. } = ev {
                assert_eq!(skill, "clock");
                saw_invoked = true;
            }
        }
        assert!(saw_invoked, "expected Event::SkillInvoked");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn skill_error_is_propagated_without_panic_event() {
        let reg = registry_with("weather", |_| {
            Err(SkillError::HttpFailed("upstream down".into()))
        });
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let (handle, task) = SkillDispatcher::spawn(reg, ev_tx.clone(), CancellationToken::new());

        let session = SessionId::new_v4();
        let err = handle
            .call(session, "weather".into(), intent("weather.query"))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::HttpFailed(ref m) if m.contains("upstream down")));

        drop(handle);
        task.await.unwrap();

        let mut saw_panicked = false;
        let mut saw_invoked = false;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Event::SkillInvoked { .. } => saw_invoked = true,
                Event::SkillPanicked { .. } => saw_panicked = true,
                _ => {}
            }
        }
        assert!(saw_invoked);
        assert!(
            !saw_panicked,
            "SkillError must NOT surface as SkillPanicked"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn panic_in_skill_becomes_skill_panicked_event() {
        let reg = registry_with("crasher", |_| {
            panic!("boom from skill");
        });
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let (handle, task) = SkillDispatcher::spawn(reg, ev_tx.clone(), CancellationToken::new());

        let session = SessionId::new_v4();
        let err = handle
            .call(session, "crasher".into(), intent("anything"))
            .await
            .unwrap_err();
        assert!(matches!(err, SkillError::Custom(ref m) if m.contains("panicked")));

        drop(handle);
        task.await.unwrap();

        let mut reason: Option<String> = None;
        while let Ok(ev) = ev_rx.try_recv() {
            if let Event::SkillPanicked {
                skill, reason: r, ..
            } = ev
            {
                assert_eq!(skill, "crasher");
                reason = Some(r);
            }
        }
        let reason = reason.expect("expected Event::SkillPanicked");
        assert!(reason.contains("boom from skill"), "reason was: {reason}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fire_and_forget_dispatch_is_observed_via_events_only() {
        let counts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counts2 = counts.clone();
        let reg = registry_with("silent", move |_| {
            counts2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(SkillResponse::empty())
        });
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let (handle, task) = SkillDispatcher::spawn(reg, ev_tx, CancellationToken::new());

        let session = SessionId::new_v4();
        handle
            .send(session, "silent".into(), intent("noop"))
            .await
            .unwrap();

        drop(handle);
        task.await.unwrap();

        assert_eq!(counts.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut saw_invoked = false;
        while let Ok(ev) = ev_rx.try_recv() {
            if matches!(ev, Event::SkillInvoked { .. }) {
                saw_invoked = true;
            }
        }
        assert!(saw_invoked);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_shuts_down_actor_even_with_pending_senders() {
        let reg = Arc::new(SkillRegistry::new());
        let (ev_tx, _ev_rx) = broadcast::channel(4);
        let cancel = CancellationToken::new();
        let (_handle, task) = SkillDispatcher::spawn(reg, ev_tx, cancel.clone());
        cancel.cancel();
        task.await.unwrap();
    }
}
