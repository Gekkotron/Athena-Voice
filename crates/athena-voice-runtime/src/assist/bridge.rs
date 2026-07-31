//! AssistBridge: consumes `{prefix}/transcription/{device}` questions and
//! answers as text on `{prefix}/tts/{device}`, with loader statuses on
//! `{prefix}/llm/{device}/status`. One actor per device, each owning a
//! router + LLM mini-pipeline (no STT/VAD/TTS actors).

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde_json::json;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::types::Transcript;
use athena_voice_providers::ProviderFactory;

use crate::assist::topics;
use crate::intent::{IntentMatcher, RuleIndex};
use crate::pipeline::llm::spawn_llm;
use crate::pipeline::router::{RouterDeps, spawn_router};
use crate::pipeline::sentence::{IDLE_FLUSH, SentenceBuffer};
use crate::wasm::dispatcher::SkillDispatcherHandle;
use crate::wasm::host_fns::MqttPublisher;

/// How long after a question we wait for the first answer text before
/// force-publishing a `done` status, so the app's loader can't get stuck.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

pub struct AssistInit {
    pub topic_prefix: String,
    pub locale: Locale,
    pub session_idle: Duration,
}

pub struct AssistDeps {
    pub publisher: Arc<dyn MqttPublisher>,
    pub factory: Arc<ProviderFactory>,
    pub matcher: Arc<IntentMatcher>,
    pub rules: Arc<ArcSwap<RuleIndex>>,
    pub dispatcher: Option<SkillDispatcherHandle>,
    pub event_bus: broadcast::Sender<Event>,
    pub shutdown: CancellationToken,
}

pub struct AssistBridge {
    init: AssistInit,
    deps: AssistDeps,
    /// device id → question channel into that device's actor.
    devices: DashMap<String, mpsc::Sender<String>>,
}

impl AssistBridge {
    #[must_use]
    pub fn new(init: AssistInit, deps: AssistDeps) -> Arc<Self> {
        Arc::new(Self {
            init,
            deps,
            devices: DashMap::new(),
        })
    }

    #[must_use]
    pub fn transcription_wildcard(&self) -> String {
        topics::transcription_wildcard(&self.init.topic_prefix)
    }

    /// Routes an MQTT publish. Returns true when the topic belongs to this
    /// bridge (even if the payload was dropped as malformed).
    pub fn handle(self: &Arc<Self>, topic: &str, payload: &[u8]) -> bool {
        let Some(device) = topics::parse_transcription(&self.init.topic_prefix, topic) else {
            return false;
        };
        let Some(text) = topics::parse_text_payload(payload) else {
            warn!(%topic, "assist: malformed or empty payload dropped");
            return true;
        };
        self.route(&device, text);
        true
    }

    fn route(self: &Arc<Self>, device: &str, text: String) {
        // Fast path: existing actor.
        if let Some(tx) = self.devices.get(device) {
            match tx.try_send(text.clone()) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    // The actor is still working through a backlog of
                    // unanswered questions — it is NOT dead. Respawning
                    // here would race a second actor against the live one
                    // over the same device, and the old actor's eventual
                    // exit would then delete the new one's map entry.
                    // Drop the question instead of respawning.
                    warn!(%device, "assist: device actor backlog full; dropping question");
                    return;
                }
                Err(TrySendError::Closed(_)) => {
                    // Actor exited (idle self-reap or channel failure);
                    // fall through and respawn below.
                }
            }
            drop(tx);
            self.devices.remove(device);
        }
        let (tx, rx) = mpsc::channel::<String>(8);
        if tx.try_send(text).is_err() {
            return; // unreachable with a fresh channel; satisfies clippy
        }
        let my_tx = tx.clone();
        self.devices.insert(device.to_string(), tx);
        self.spawn_device_actor(device.to_string(), my_tx, rx);
    }

    fn spawn_device_actor(
        self: &Arc<Self>,
        device: String,
        my_tx: mpsc::Sender<String>,
        question_rx: mpsc::Receiver<String>,
    ) {
        let bridge = self.clone();
        drop(tokio::spawn(async move {
            bridge.run_device_actor(device, my_tx, question_rx).await;
        }));
    }

    async fn run_device_actor(
        self: Arc<Self>,
        device: String,
        my_tx: mpsc::Sender<String>,
        mut question_rx: mpsc::Receiver<String>,
    ) {
        let sid = SessionId::new_v4();
        let cancel = self.deps.shutdown.child_token();
        let prefix = self.init.topic_prefix.clone();
        let tts_topic = topics::tts_topic(&prefix, &device);
        let status_topic = topics::status_topic(&prefix, &device);

        // Mini-pipeline: transcripts → router → (skill | LLM) → tokens.
        let (t_tx, t_rx) = mpsc::channel::<Transcript>(16);
        let (llm_prompt_tx, llm_prompt_rx) = mpsc::channel::<String>(4);
        let (tok_tx, mut tok_rx) = mpsc::channel::<String>(64);
        spawn_router(
            t_rx,
            RouterDeps {
                llm_tx: llm_prompt_tx,
                tts_tok_tx: tok_tx.clone(),
                event_tx: self.deps.event_bus.clone(),
                session: sid,
                locale: self.init.locale.clone(),
                matcher: self.deps.matcher.clone(),
                rules: self.deps.rules.clone(),
                dispatcher: self.deps.dispatcher.clone(),
            },
            cancel.clone(),
        );
        spawn_llm(
            sid,
            self.init.locale.clone(),
            self.deps.factory.llm(),
            llm_prompt_rx,
            tok_tx,
            cancel.clone(),
        );

        info!(%device, session = %sid, "assist: device session opened");
        let mut barge_rx = self.deps.event_bus.subscribe();
        let mut buf = SentenceBuffer::new();
        // Bumped on every question; tags `answer_deadline` so a sentence
        // flushed for a superseded utterance can't consume a newer
        // question's pending `done` (see the drains below and the epoch
        // check in `publish_answer`).
        let mut epoch: u64 = 0;
        // Some((epoch, deadline)) while a question awaits its first answer
        // text.
        let mut answer_deadline: Option<(u64, tokio::time::Instant)> = None;
        // Some(deadline) while `buf` holds an unpunctuated fragment waiting
        // for the idle flush. Anchored to a fixed instant — rebuilt from
        // `tokio::time::sleep(IDLE_FLUSH)` on every `select!` iteration, it
        // would never elapse under any unrelated event-bus traffic (every
        // session's events wake `barge_rx.recv()` and re-enter the loop).
        let mut flush_deadline: Option<tokio::time::Instant> = None;
        let mut idle_deadline = tokio::time::Instant::now() + self.init.session_idle;

        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep_until(idle_deadline) => {
                    info!(%device, session = %sid, "assist: device session idle, closing");
                    break;
                }
                () = async {
                    match answer_deadline {
                        Some((_, d)) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    warn!(%device, "assist: no answer within timeout; releasing loader");
                    self.publish_status(&status_topic, "done").await;
                    answer_deadline = None;
                }
                ev = barge_rx.recv() => {
                    if matches!(ev, Ok(Event::BargeIn { session, .. }) if session == sid) {
                        // Drop the buffered fragment: the previous response
                        // is dead. NB: we deliberately do NOT also drain
                        // `tok_rx` here — the router emits BargeIn on every
                        // second-and-later question in a session (it only
                        // tracks "was prior work forwarded", not "has it
                        // finished"), including the common case where Q1's
                        // answer already completed. This event can arrive
                        // after Q2's own LLM tokens have already started
                        // flowing; draining here would risk dropping THIS
                        // question's legitimate answer. The per-question
                        // drain in the `question_rx` arm below is the
                        // deterministic guard against stale content (C5).
                        buf.clear();
                        flush_deadline = None;
                    }
                    // Lagged/closed/other events: ignore.
                }
                () = async {
                    match flush_deadline {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    flush_deadline = None;
                    if let Some(sentence) = buf.take() {
                        self.publish_answer(&tts_topic, &status_topic, &sentence, epoch, &mut answer_deadline).await;
                    }
                }
                maybe = question_rx.recv() => {
                    let Some(text) = maybe else { break };
                    idle_deadline = tokio::time::Instant::now() + self.init.session_idle;
                    epoch = epoch.saturating_add(1);
                    // A new question supersedes anything still in flight
                    // for the previous one: drop the buffered fragment and
                    // any tokens the LLM had already queued before we ever
                    // forward this transcript, so they can't surface as
                    // (part of) this question's answer.
                    buf.clear();
                    flush_deadline = None;
                    while tok_rx.try_recv().is_ok() {}
                    answer_deadline = Some((epoch, tokio::time::Instant::now() + ANSWER_TIMEOUT));
                    self.publish_status(&status_topic, "in progress").await;
                    let _ = self.deps.event_bus.send(Event::TranscriptFinal {
                        session: sid,
                        text: text.clone(),
                    });
                    if t_tx.send(Transcript { text, is_final: true, confidence: None }).await.is_err() {
                        break; // router gone; actor is useless
                    }
                }
                maybe = tok_rx.recv() => {
                    let Some(tok) = maybe else {
                        if let Some(sentence) = buf.take() {
                            self.publish_answer(&tts_topic, &status_topic, &sentence, epoch, &mut answer_deadline).await;
                        }
                        break;
                    };
                    match buf.push(&tok) {
                        Some(sentence) => {
                            flush_deadline = None;
                            self.publish_answer(&tts_topic, &status_topic, &sentence, epoch, &mut answer_deadline).await;
                        }
                        None => {
                            flush_deadline = Some(tokio::time::Instant::now() + IDLE_FLUSH);
                        }
                    }
                }
            }
        }

        // A question can be mid-flight (status "in progress" published, no
        // answer yet) on every exit path above — shutdown, idle reap, or a
        // dead downstream channel. Release the app's loader unconditionally
        // rather than leaving it spinning forever.
        if answer_deadline.take().is_some() {
            self.publish_status(&status_topic, "done").await;
        }

        // Remove ONLY our own entry. If we had already been replaced (a
        // caller saw our channel Closed and respawned a fresh actor) the
        // map points at the successor — blindly removing here would delete
        // a live actor's entry instead of our own stale one.
        self.devices
            .remove_if(&device, |_, tx| tx.same_channel(&my_tx));

        // Anything that arrived in the reap window between our last
        // `question_rx.recv()` and the removal above looked routable (the
        // map entry was still live), so a caller's `try_send` succeeded
        // into a receiver nobody will ever poll again. Drain it.
        question_rx.close();
        let mut leftover = Vec::new();
        while let Ok(text) = question_rx.try_recv() {
            leftover.push(text);
        }
        if !leftover.is_empty() {
            if self.deps.shutdown.is_cancelled() {
                // Global shutdown: re-routing would spawn a fresh actor
                // whose own (child) cancel token is already cancelled,
                // which immediately re-exits into this same drain —
                // unbounded spawn churn. Drop the leftovers; the
                // pending-`done` publish above already released the
                // loader.
                warn!(
                    %device,
                    count = leftover.len(),
                    "assist: dropping leftover question(s) during shutdown"
                );
            } else {
                // Non-shutdown exit (idle reap, dead downstream channel):
                // our entry is gone, so routing spawns a fresh actor to
                // answer it.
                for text in leftover {
                    self.route(&device, text);
                }
            }
        }

        cancel.cancel();
        info!(%device, session = %sid, "assist: device session closed");
    }

    async fn publish_answer(
        &self,
        tts_topic: &str,
        status_topic: &str,
        sentence: &str,
        epoch: u64,
        answer_deadline: &mut Option<(u64, tokio::time::Instant)>,
    ) {
        let payload = json!({ "text": sentence }).to_string();
        if let Err(e) = self
            .deps
            .publisher
            .publish(tts_topic.to_string(), payload.into_bytes())
            .await
        {
            warn!(error = %e, "assist: answer publish failed");
        }
        // First answer text FOR THIS QUESTION releases the app's loader.
        // The epoch tag stops a flush left over from a superseded
        // utterance from consuming a newer question's pending `done`.
        if answer_deadline.as_ref().is_some_and(|(e, _)| *e == epoch) {
            *answer_deadline = None;
            self.publish_status(status_topic, "done").await;
        }
    }

    async fn publish_status(&self, status_topic: &str, status: &str) {
        let payload = json!({ "status": status }).to_string();
        if let Err(e) = self
            .deps
            .publisher
            .publish(status_topic.to_string(), payload.into_bytes())
            .await
        {
            warn!(error = %e, "assist: status publish failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use arc_swap::ArcSwap;
    use tokio::sync::broadcast;
    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    use athena_voice_core::ids::Locale;
    use athena_voice_core::provider::{BoxError, CompletionStream, Llm};
    use athena_voice_providers::{ProviderConfig, ProviderFactory, StageChoice};

    use crate::intent::{IntentMatcher, RuleIndex};
    use crate::wasm::host_fns::MqttPublisher;

    /// Records publishes and wakes waiters.
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
            self.wait_for_within(Duration::from_secs(5), pred).await;
        }

        /// Same as `wait_for` with a caller-supplied cap — needed for the
        /// `ANSWER_TIMEOUT` test, whose 30 s virtual wait would otherwise
        /// trip the default 5 s cap before the timeout itself fires.
        async fn wait_for_within(&self, cap: Duration, pred: impl Fn(&[(String, String)]) -> bool) {
            timeout(cap, async {
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

    /// An `Llm` that never produces a token — used to pin the
    /// `ANSWER_TIMEOUT` fallback path, where no answer ever arrives.
    struct SilentLlm;

    #[async_trait::async_trait]
    impl Llm for SilentLlm {
        async fn complete(
            &self,
            _session: SessionId,
            _locale: Locale,
            _prompt: String,
            _history: Vec<(String, String)>,
        ) -> Result<CompletionStream, BoxError> {
            Ok(Box::pin(futures::stream::empty()))
        }

        fn name(&self) -> &'static str {
            "silent"
        }
    }

    async fn build_bridge(
        publisher: Arc<RecordingPublisher>,
        session_idle: Duration,
        llm_override: Option<Arc<dyn Llm>>,
        shutdown: CancellationToken,
    ) -> Arc<AssistBridge> {
        let mut factory = ProviderFactory::build(
            &ProviderConfig {
                stt: StageChoice::Fake,
                llm: StageChoice::Fake,
                tts: StageChoice::Fake,
            },
            None,
        )
        .await
        .unwrap();
        if let Some(llm) = llm_override {
            factory = factory.with_llm(llm);
        }
        let factory = Arc::new(factory);
        let (event_tx, _rx) = broadcast::channel(64);
        AssistBridge::new(
            AssistInit {
                topic_prefix: "assist".into(),
                locale: Locale::new("fr").unwrap(),
                session_idle,
            },
            AssistDeps {
                publisher,
                factory,
                matcher: Arc::new(IntentMatcher::new()),
                rules: Arc::new(ArcSwap::from_pointee(RuleIndex::new())),
                dispatcher: None,
                event_bus: event_tx,
                shutdown,
            },
        )
    }

    async fn bridge_with(publisher: Arc<RecordingPublisher>) -> Arc<AssistBridge> {
        build_bridge(
            publisher,
            Duration::from_secs(120),
            None,
            CancellationToken::new(),
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn question_produces_status_answer_done() {
        let publisher = RecordingPublisher::new();
        let bridge = bridge_with(publisher.clone()).await;

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));

        // in-progress status precedes the answer.
        publisher
            .wait_for(|p| {
                p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
            })
            .await;
        // Fake LLM answer arrives as text on the tts topic.
        publisher
            .wait_for(|p| p.iter().any(|(t, _)| t == "assist/tts/pixel"))
            .await;
        // done status follows the answer.
        publisher
            .wait_for(|p| {
                p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
            })
            .await;
        // Give any incorrect extra publish a moment to show up before
        // pinning the final shape below.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let published = publisher.published.lock().unwrap().clone();

        // Shapes: answers are {"text": ...}, statuses are {"status": ...}.
        let answer_idx = published
            .iter()
            .position(|(t, _)| t == "assist/tts/pixel")
            .unwrap();
        let in_progress_idx = published
            .iter()
            .position(|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
            .unwrap();
        let done_positions: Vec<usize> = published
            .iter()
            .enumerate()
            .filter(|(_, (t, m))| t == "assist/llm/pixel/status" && m.contains("done"))
            .map(|(i, _)| i)
            .collect();

        assert_eq!(
            done_positions.len(),
            1,
            "done must be published exactly once; got {done_positions:?} in {published:?}"
        );
        assert!(
            in_progress_idx < answer_idx,
            "in-progress must precede the answer: {published:?}"
        );
        assert!(
            answer_idx <= done_positions[0],
            "answer must precede (or coincide with) done: {published:?}"
        );

        let v: serde_json::Value = serde_json::from_str(&published[answer_idx].1).unwrap();
        assert!(
            v.get("text")
                .and_then(|t| t.as_str())
                .is_some_and(|s| !s.is_empty())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foreign_and_malformed_messages_are_ignored() {
        let publisher = RecordingPublisher::new();
        let bridge = bridge_with(publisher.clone()).await;

        assert!(!bridge.handle("athena/sat/x/session/y/text", b"hello"));
        assert!(!bridge.handle("assist/tts/pixel", br#"{"text": "loop!"}"#));
        // Consumed (it IS our topic) but dropped: malformed payload.
        assert!(bridge.handle("assist/transcription/pixel", b"not json"));
        assert!(bridge.handle("assist/transcription/+", br#"{"text": "x"}"#) == false);

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            publisher.published.lock().unwrap().is_empty(),
            "nothing may be published for ignored input"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn idle_device_is_reaped_then_a_fresh_question_recreates_it() {
        let publisher = RecordingPublisher::new();
        // `idle_deadline` resets only on question arrival, not on answer
        // completion, so this must comfortably exceed `IDLE_FLUSH` (800 ms)
        // or the actor would reap before its own answer ever flushes.
        let bridge = build_bridge(
            publisher.clone(),
            Duration::from_secs(2),
            None,
            CancellationToken::new(),
        )
        .await;

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));
        publisher
            .wait_for(|p| p.iter().any(|(t, _)| t == "assist/tts/pixel"))
            .await;

        // Wait past the idle timeout (measured from the question, not the
        // answer) for the actor to self-reap.
        tokio::time::sleep(Duration::from_millis(2200)).await;
        assert_eq!(
            bridge.devices.len(),
            0,
            "idle actor must remove its own map entry"
        );

        // A fresh question must spin up a brand-new actor and answer
        // normally — proving recreation after reap actually works.
        let before = publisher.published.lock().unwrap().len();
        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));
        publisher
            .wait_for(|p| {
                p.len() > before && p[before..].iter().any(|(t, _)| t == "assist/tts/pixel")
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn no_answer_within_timeout_still_releases_the_loader() {
        let publisher = RecordingPublisher::new();
        let bridge = build_bridge(
            publisher.clone(),
            Duration::from_secs(120),
            Some(Arc::new(SilentLlm)),
            CancellationToken::new(),
        )
        .await;

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));

        publisher
            .wait_for(|p| {
                p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress"))
            })
            .await;

        // The LLM never produces a single token, so only the 30 s
        // ANSWER_TIMEOUT can release the loader. `start_paused` fast-forwards
        // the virtual clock instead of a real 30 s wait.
        publisher
            .wait_for_within(Duration::from_secs(35), |p| {
                p.iter()
                    .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
            })
            .await;

        let published = publisher.published.lock().unwrap().clone();
        assert!(
            !published.iter().any(|(t, _)| t == "assist/tts/pixel"),
            "no answer text should ever be published: {published:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_question_gets_its_own_answer_and_done_without_interleaving() {
        let publisher = RecordingPublisher::new();
        let bridge = bridge_with(publisher.clone()).await;

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));
        publisher
            .wait_for(|p| p.iter().any(|(t, _)| t == "assist/tts/pixel"))
            .await;

        let before = publisher.published.lock().unwrap().len();

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il encore"}"#
        ));
        publisher
            .wait_for(|p| {
                p.len() > before
                    && p[before..]
                        .iter()
                        .any(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
            })
            .await;
        // Give any incorrect extra publish a moment to show up.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let published = publisher.published.lock().unwrap().clone();
        let done_count = published
            .iter()
            .filter(|(t, m)| t == "assist/llm/pixel/status" && m.contains("done"))
            .count();
        assert_eq!(
            done_count, 2,
            "each question must release exactly one done: {published:?}"
        );

        // Everything from `before` onward belongs to Q2 alone: it must
        // open with its own in-progress and contain exactly one done — no
        // stray content leaked in from Q1's superseded stream.
        let second_batch = &published[before..];
        assert!(
            matches!(second_batch.first(), Some((t, m)) if t == "assist/llm/pixel/status" && m.contains("in progress")),
            "Q2 must open with its own in-progress status: {second_batch:?}"
        );
        let second_done_positions: Vec<usize> = second_batch
            .iter()
            .enumerate()
            .filter(|(_, (t, m))| t == "assist/llm/pixel/status" && m.contains("done"))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            second_done_positions.len(),
            1,
            "Q2 must publish exactly one done: {second_batch:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_drops_leftovers_instead_of_respawn_churn() {
        let publisher = RecordingPublisher::new();
        let shutdown = CancellationToken::new();
        let bridge = build_bridge(
            publisher.clone(),
            Duration::from_secs(120),
            None,
            shutdown.clone(),
        )
        .await;

        assert!(bridge.handle(
            "assist/transcription/pixel",
            br#"{"text": "quelle heure est-il"}"#
        ));
        // Cancel immediately — before the actor is guaranteed to have even
        // taken its first `select!` turn, so its own `cancel` token (a
        // child of `shutdown`) is very likely already cancelled by the
        // time it runs, exercising exactly the shutdown-vs-question_rx
        // race the fix targets.
        shutdown.cancel();

        // Give everything (actor exit, any leftover drop/warn, any
        // pending-done publish) time to settle.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            bridge.devices.len(),
            0,
            "shutdown must leave no live device entries"
        );
        let settled = publisher.published.lock().unwrap().len();

        // Without the fix, a leftover question re-routed during shutdown
        // spawns a fresh actor whose own child token is already
        // cancelled, which (depending on `select!`'s race) can publish
        // another spurious in-progress/done before repeating the same
        // drain — churning indefinitely. Assert the publish count has
        // genuinely stabilized rather than still growing.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let still = publisher.published.lock().unwrap().len();
        assert_eq!(
            settled, still,
            "no further publishes should occur after shutdown settles — spawn churn detected"
        );
        assert_eq!(bridge.devices.len(), 0);
    }
}
