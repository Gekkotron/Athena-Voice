# Assist Bridge + GEEKOM Deployment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Athena answers text questions from the owner's Android DomoticApp via MQTT topics `assist/transcription/{device}` → `assist/tts/{device}`, running as a single audio-free `serve` process on a Linux GEEKOM.

**Architecture:** A new `assist` module in `athena-voice-runtime` subscribes to the app's existing topics on the LAN broker, opens a per-device mini-pipeline (router + LLM only — no STT/VAD/TTS actors), consumes the answer token channel directly, and publishes sentences as JSON text plus loader-status messages. Sentence splitting is extracted from the TTS actor into a shared `SentenceBuffer`.

**Tech Stack:** Rust (workspace, toolchain 1.91), tokio, rumqttc, serde_json, figment (config), extism (WASM skills, tests only), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-07-31-assist-bridge-geekom-design.md` (including its "Implementation notes" corrections).

## Global Constraints

- `athena-voice-runtime/src/lib.rs` has `#![deny(warnings)]`; CI runs `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo fmt --all --check`. Run both before every commit.
- Tests run with `cargo nextest run -p athena-voice-runtime` (fast) and `cargo nextest run --workspace` (before finishing a task).
- The app's wire shapes are FIXED (the app is not being changed): inbound `{"text": "..."}` on `assist/transcription/{device}`; outbound `{"text": "..."}` on `assist/tts/{device}`; status `{"status": "in progress"}` / `{"status": "done"}` on `assist/llm/{device}/status`.
- Existing satellite/session wire protocols must not change.
- Commit as Gekkotron on every commit: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit ...`, message ending with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Do not commit `skills-*/target/`, `*.wasm` artifacts, or `models/` downloads.

## File Structure

- Fix: `.github/workflows/ci.yml` (invalid YAML, every run fails)
- Create: `crates/athena-voice-runtime/src/pipeline/sentence.rs` — pure `SentenceBuffer`
- Modify: `crates/athena-voice-runtime/src/pipeline/tts.rs` — use `SentenceBuffer`
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs` — export `sentence`
- Create: `crates/athena-voice-runtime/src/assist/mod.rs` — module root + `AssistInit`
- Create: `crates/athena-voice-runtime/src/assist/topics.rs` — topic parse/build + device-id validation
- Create: `crates/athena-voice-runtime/src/assist/bridge.rs` — `AssistBridge` ingress + per-device session actor
- Modify: `crates/athena-voice-runtime/src/lib.rs` — `Runtime::spawn` gains `assist: Option<AssistInit>`
- Modify: `crates/athena-voice-runtime/src/satellite/ingress.rs` — route assist topics to the bridge
- Modify: `crates/athena-voice-cli/src/config.rs` — `[assist]` block
- Modify: `crates/athena-voice-cli/src/serve.rs` — plumb `[assist]` into `Runtime::spawn`
- Modify: `crates/athena-voice-runtime/tests/runtime_spawn.rs` — new `Runtime::spawn` arg
- Create: `crates/athena-voice-runtime/tests/assist_end_to_end.rs` — integration tests
- Create: `athena.assist.toml` — GEEKOM profile
- Modify: `README.md` — "Run on a Linux box (GEEKOM)" + DomoticApp protocol section
- Modify: `PLAN.md` — Done entry at the end

---

### Task 1: Repair the CI workflow

CI is red on every push: `.github/workflows/ci.yml` line 61 has a `- run:` step indented outside the `steps:` list of the `test` job, which is invalid YAML, so the entire workflow fails immediately. All jobs already run on `ubuntu-latest`, so a green CI *is* the spec's "Linux proof".

**Files:**
- Modify: `.github/workflows/ci.yml:60-61`

**Interfaces:**
- Produces: a parseable workflow; later tasks rely on CI to prove Linux builds.

- [ ] **Step 1: Look at the broken lines**

Current content (note the last line's indentation — it sits outside the `steps:` sequence):

```yaml
      - run: cargo nextest run --workspace --profile ci
    - run: cargo build --package athena-voice-server --package athena-voice-client
```

- [ ] **Step 2: Fix the indentation**

Replace those two lines with:

```yaml
      - run: cargo nextest run --workspace --profile ci
      - run: cargo build --package athena-voice-server --package athena-voice-client
```

- [ ] **Step 3: Validate the YAML locally**

Run: `ruby -e "require 'yaml'; YAML.load_file('.github/workflows/ci.yml'); puts 'valid'"`
Expected: `valid` (before the fix this prints a Psych syntax error — run it before AND after to see the failure first).

- [ ] **Step 4: Check `.config/nextest.toml` defines the `ci` profile**

Run: `grep -n "profile.ci" .config/nextest.toml`
Expected: a `[profile.ci]` section exists. If it does not, change the workflow line to `cargo nextest run --workspace` (drop `--profile ci`) instead of inventing a profile.

- [ ] **Step 5: Commit and push, then watch the run**

```bash
git add .github/workflows/ci.yml
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "CI: fix step indentation that invalidated the workflow YAML

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
gh run watch --repo Gekkotron/Athena-Voice --exit-status || gh run list --repo Gekkotron/Athena-Voice --limit 1
```

Expected: the workflow parses and jobs actually start. If a job fails for a pre-existing reason unrelated to YAML (e.g. the coverage job's codecov token), note it and continue — the `test` (nextest) job going green is the deliverable. If `test` itself fails on a real Linux incompatibility, STOP and report; that's exactly what this task exists to surface.

---

### Task 2: Extract `SentenceBuffer` from the TTS actor

The TTS actor buffers streamed tokens and flushes on sentence boundary (`.`/`!`/`?`), on a 100-char cap, or on 800 ms idle. The assist bridge needs identical semantics, so the pure buffering moves to `pipeline/sentence.rs`; the select-loop skeletons stay separate.

**Files:**
- Create: `crates/athena-voice-runtime/src/pipeline/sentence.rs`
- Modify: `crates/athena-voice-runtime/src/pipeline/tts.rs` (replace `buf: String` + `is_sentence_boundary` with the buffer)
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs` (add `pub mod sentence;`)

**Interfaces:**
- Produces:
  - `pub struct SentenceBuffer` with `pub fn new() -> Self` (and `Default`),
    `pub fn push(&mut self, token: &str) -> Option<String>` (returns a trimmed sentence when the token completes one),
    `pub fn take(&mut self) -> Option<String>` (drains the remainder, used by idle flush / channel close; `None` if only whitespace),
    `pub fn clear(&mut self)` (barge-in), `pub fn is_empty(&self) -> bool` (true when nothing non-whitespace is buffered).
  - `pub const IDLE_FLUSH: std::time::Duration` moves into `sentence.rs` (re-export or import in `tts.rs`).

- [ ] **Step 1: Write the failing tests**

Create `crates/athena-voice-runtime/src/pipeline/sentence.rs` containing ONLY the tests module for now (plus an empty struct so it compiles — no, per TDD write tests against the intended API and let the compile failure be the RED):

```rust
//! Sentence aggregation shared by the TTS actor and the assist bridge.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_returns_sentence_on_boundary() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.push("Bonjour"), None);
        assert_eq!(b.push("."), Some("Bonjour.".to_string()));
        assert!(b.is_empty());
    }

    #[test]
    fn push_flushes_on_length_cap() {
        let mut b = SentenceBuffer::new();
        let long = "a".repeat(100);
        let flushed = b.push(&long);
        assert_eq!(flushed, Some(long));
    }

    #[test]
    fn tokens_are_verbatim_no_separator_injected() {
        // LLMs stream sub-word pieces ("Le", " temps") that own their spacing.
        let mut b = SentenceBuffer::new();
        b.push("Le");
        b.push(" temps");
        assert_eq!(b.push("."), Some("Le temps.".to_string()));
    }

    #[test]
    fn take_drains_unpunctuated_remainder() {
        let mut b = SentenceBuffer::new();
        b.push("je ne sais pas");
        assert_eq!(b.take(), Some("je ne sais pas".to_string()));
        assert!(b.is_empty());
        assert_eq!(b.take(), None);
    }

    #[test]
    fn take_on_whitespace_only_is_none() {
        let mut b = SentenceBuffer::new();
        b.push("   ");
        assert_eq!(b.take(), None);
        assert!(b.is_empty());
    }

    #[test]
    fn clear_drops_buffered_text() {
        let mut b = SentenceBuffer::new();
        b.push("Bonjour");
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.push("Nouveau."), Some("Nouveau.".to_string()));
    }
}
```

Add `pub mod sentence;` to `crates/athena-voice-runtime/src/pipeline/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p athena-voice-runtime sentence 2>&1 | tail -5`
Expected: COMPILE ERROR — `SentenceBuffer` not found. That is the RED for a new type.

- [ ] **Step 3: Implement `SentenceBuffer`**

Above the tests module in `sentence.rs`:

```rust
/// Buffered text with no sentence boundary is flushed after this much
/// token-channel silence — LLM answers don't reliably end in punctuation
/// ("je ne sais pas"), and without this they would never be spoken.
pub const IDLE_FLUSH: std::time::Duration = std::time::Duration::from_millis(800);

fn is_sentence_boundary(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// Aggregates verbatim token fragments into sentences. Producers own their
/// spacing (LLMs stream sub-word pieces), so no separators are inserted.
/// Flushes on `.`/`!`/`?` or once the buffer reaches 100 bytes.
#[derive(Default)]
pub struct SentenceBuffer {
    buf: String,
}

impl SentenceBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `token`; returns a completed sentence when this token ends
    /// one (boundary char or length cap), trimmed.
    pub fn push(&mut self, token: &str) -> Option<String> {
        self.buf.push_str(token);
        let boundary = self
            .buf
            .trim_end()
            .chars()
            .last()
            .is_some_and(is_sentence_boundary);
        if (boundary || self.buf.len() >= 100) && !self.buf.trim().is_empty() {
            let out = self.buf.trim().to_string();
            self.buf.clear();
            return Some(out);
        }
        None
    }

    /// Drains whatever is buffered (idle flush / end of stream).
    pub fn take(&mut self) -> Option<String> {
        let out = self.buf.trim().to_string();
        self.buf.clear();
        if out.is_empty() { None } else { Some(out) }
    }

    /// Drops buffered text (barge-in: the pending response is dead).
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.trim().is_empty()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p athena-voice-runtime sentence 2>&1 | tail -5`
Expected: all 6 PASS.

- [ ] **Step 5: Refactor `tts.rs` onto the buffer (behavior-preserving)**

In `crates/athena-voice-runtime/src/pipeline/tts.rs`:
- Delete `fn is_sentence_boundary` and the local `const IDLE_FLUSH` (import `crate::pipeline::sentence::{SentenceBuffer, IDLE_FLUSH}` instead).
- Replace `let mut buf = String::new();` with `let mut buf = SentenceBuffer::new();`.
- Barge-in arm: `buf.clear();` stays as is.
- Idle-flush arm becomes:

```rust
() = tokio::time::sleep(IDLE_FLUSH), if !buf.is_empty() => {
    if let Some(sentence) = buf.take() {
        seq = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
    }
}
```

- Token arm becomes:

```rust
maybe = token_rx.recv() => {
    let Some(tok) = maybe else {
        if let Some(sentence) = buf.take() {
            let _ = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
        }
        break;
    };
    if let Some(sentence) = buf.push(&tok) {
        seq = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
    }
}
```

- [ ] **Step 6: Run the TTS actor's existing tests — they pin the behavior**

Run: `cargo nextest run -p athena-voice-runtime pipeline::tts 2>&1 | tail -5` then `cargo nextest run -p athena-voice-runtime 2>&1 | tail -3`
Expected: `idle_flush_speaks_unpunctuated_answers`, `buffers_by_sentence_and_synthesises_each`, `barge_in_flushes_buffered_text_before_synthesis` all PASS, plus the rest of the crate.

- [ ] **Step 7: Lint and commit**

```bash
cargo fmt --all && cargo clippy -p athena-voice-runtime --all-targets -- -D warnings
git add crates/athena-voice-runtime/src/pipeline/
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Extract SentenceBuffer from the TTS actor

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Assist topic parsing and validation

Pure functions for the assist wire: parse an inbound transcription topic into a device id (rejecting hostile ids), build outbound topics, parse the inbound payload.

**Files:**
- Create: `crates/athena-voice-runtime/src/assist/mod.rs`
- Create: `crates/athena-voice-runtime/src/assist/topics.rs`
- Modify: `crates/athena-voice-runtime/src/lib.rs` (add `pub mod assist;` to the module list)

**Interfaces:**
- Produces (all in `crate::assist::topics`):
  - `pub fn transcription_wildcard(prefix: &str) -> String` → `"{prefix}/transcription/+"`
  - `pub fn parse_transcription(prefix: &str, topic: &str) -> Option<String>` → device id, `None` for foreign/invalid topics
  - `pub fn tts_topic(prefix: &str, device: &str) -> String` → `"{prefix}/tts/{device}"`
  - `pub fn status_topic(prefix: &str, device: &str) -> String` → `"{prefix}/llm/{device}/status"`
  - `pub fn parse_text_payload(payload: &[u8]) -> Option<String>` → non-empty trimmed `text` field
- `assist/mod.rs` starts as:

```rust
//! Assist bridge: text questions from the owner's home-automation app
//! (topics `assist/transcription/{device}`) answered as text on
//! `assist/tts/{device}`. See docs/superpowers/specs/2026-07-31-*.md.

pub mod topics;
```

- [ ] **Step 1: Write the failing tests**

`crates/athena-voice-runtime/src/assist/topics.rs`, tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_shape() {
        assert_eq!(transcription_wildcard("assist"), "assist/transcription/+");
    }

    #[test]
    fn parse_extracts_device() {
        assert_eq!(
            parse_transcription("assist", "assist/transcription/pixel-7"),
            Some("pixel-7".to_string())
        );
    }

    #[test]
    fn parse_rejects_foreign_topics() {
        assert_eq!(parse_transcription("assist", "assist/tts/pixel-7"), None);
        assert_eq!(parse_transcription("assist", "athena/sat/x/session/y/text"), None);
        assert_eq!(parse_transcription("assist", "assist/transcription"), None);
        // Extra levels would mean the device id contained '/': reject.
        assert_eq!(parse_transcription("assist", "assist/transcription/a/b"), None);
    }

    #[test]
    fn parse_rejects_hostile_device_ids() {
        // '+'/'#' in a concrete (non-filter) topic are legal MQTT but would
        // let a publisher steer our answer topic — reject.
        assert_eq!(parse_transcription("assist", "assist/transcription/+"), None);
        assert_eq!(parse_transcription("assist", "assist/transcription/#"), None);
        assert_eq!(parse_transcription("assist", "assist/transcription/"), None);
    }

    #[test]
    fn outbound_topics() {
        assert_eq!(tts_topic("assist", "pixel"), "assist/tts/pixel");
        assert_eq!(status_topic("assist", "pixel"), "assist/llm/pixel/status");
    }

    #[test]
    fn payload_requires_nonempty_text() {
        assert_eq!(
            parse_text_payload(br#"{"text": "quelle heure est-il"}"#),
            Some("quelle heure est-il".to_string())
        );
        assert_eq!(parse_text_payload(br#"{"text": "  "}"#), None);
        assert_eq!(parse_text_payload(br#"{"other": 1}"#), None);
        assert_eq!(parse_text_payload(b"not json"), None);
        // Trims surrounding whitespace.
        assert_eq!(
            parse_text_payload(br#"{"text": " bonjour "}"#),
            Some("bonjour".to_string())
        );
    }
}
```

Wire the module: add `pub mod topics;` via the new `assist/mod.rs`, and `pub mod assist;` in `lib.rs` (alphabetical: after `pub mod audio;`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p athena-voice-runtime assist::topics 2>&1 | tail -5`
Expected: COMPILE ERROR — functions not defined.

- [ ] **Step 3: Implement**

Above the tests in `topics.rs`:

```rust
//! Topic layout and payload shapes for the assist bridge. The wire is FIXED
//! by the DomoticApp side: `{prefix}/transcription/{device}` in,
//! `{prefix}/tts/{device}` + `{prefix}/llm/{device}/status` out, all JSON.

#[must_use]
pub fn transcription_wildcard(prefix: &str) -> String {
    format!("{prefix}/transcription/+")
}

/// Extracts the device id from `{prefix}/transcription/{device}`. Returns
/// `None` for foreign topics and for device ids that are empty or contain
/// MQTT-special characters (`/`, `+`, `#`) — those would let a hostile
/// publisher steer the answer topic.
#[must_use]
pub fn parse_transcription(prefix: &str, topic: &str) -> Option<String> {
    let rest = topic.strip_prefix(prefix)?.strip_prefix('/')?;
    let device = rest.strip_prefix("transcription/")?;
    if device.is_empty() || device.contains(['/', '+', '#']) {
        return None;
    }
    Some(device.to_string())
}

#[must_use]
pub fn tts_topic(prefix: &str, device: &str) -> String {
    format!("{prefix}/tts/{device}")
}

#[must_use]
pub fn status_topic(prefix: &str, device: &str) -> String {
    format!("{prefix}/llm/{device}/status")
}

/// Parses the app's `{"text": "..."}` payload; `None` unless `text` is a
/// non-empty string after trimming.
#[must_use]
pub fn parse_text_payload(payload: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let text = v.get("text")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p athena-voice-runtime assist::topics 2>&1 | tail -5`
Expected: all 6 PASS.

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --all && cargo clippy -p athena-voice-runtime --all-targets -- -D warnings
git add crates/athena-voice-runtime/src/assist/ crates/athena-voice-runtime/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Assist bridge: topic layout, device-id validation, payload parsing

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: The `AssistBridge` — ingress + per-device session actor

The heart of the feature. `AssistBridge::handle(topic, payload)` is called from the satellite MQTT loop for assist topics; it validates, then routes the question into a per-device actor. Each actor owns a router+LLM mini-pipeline (no STT/VAD/TTS) and publishes answer sentences + statuses through `Arc<dyn MqttPublisher>` (the trait from `wasm::host_fns`, so tests inject a recorder).

**Files:**
- Create: `crates/athena-voice-runtime/src/assist/bridge.rs`
- Modify: `crates/athena-voice-runtime/src/assist/mod.rs` (add `pub mod bridge;` and re-export `pub use bridge::{AssistBridge, AssistDeps, AssistInit};`)

**Interfaces:**
- Consumes: `SentenceBuffer`/`IDLE_FLUSH` (Task 2), `assist::topics` (Task 3), `MqttPublisher` (`crate::wasm::host_fns`), `RouterDeps`/`spawn_router` (`crate::pipeline::router`), `spawn_llm` (`crate::pipeline::llm`), `IntentMatcher`/`RuleIndex` (`crate::intent`), `SkillDispatcherHandle` (`crate::wasm::dispatcher`), `ProviderFactory` (athena_voice_providers), `Event`/`Locale`/`SessionId`/`Transcript` (athena_voice_core).
- Produces:

```rust
pub struct AssistInit {
    pub topic_prefix: String,          // default "assist"
    pub locale: Locale,
    pub session_idle: std::time::Duration,
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

pub struct AssistBridge { /* private fields */ }
impl AssistBridge {
    pub fn new(init: AssistInit, deps: AssistDeps) -> Arc<Self>;
    pub fn transcription_wildcard(&self) -> String;
    /// True if the topic belongs to this bridge (was consumed).
    pub fn handle(self: &Arc<Self>, topic: &str, payload: &[u8]) -> bool;
}
```

- [ ] **Step 1: Write the failing test — happy path with a recording publisher**

The test wires a REAL router (matcher with no rules, no dispatcher) and the FAKE LLM provider, so an unmatched question flows router → LLM → tokens → bridge publishes. Check what `StageChoice::Fake` LLM streams by reading `crates/athena-voice-providers/src/testing/` (fake LLM) FIRST — assert on its actual deterministic output, not a guess. The assertions below use "ANSWER" as a stand-in: replace it with the fake's real text once read.

In `bridge.rs`:

```rust
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

        /// Waits until `pred` holds over the published list (5 s cap).
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

    async fn bridge_with(publisher: Arc<RecordingPublisher>) -> Arc<AssistBridge> {
        let factory = Arc::new(
            ProviderFactory::build(
                &ProviderConfig {
                    stt: StageChoice::Fake,
                    llm: StageChoice::Fake,
                    tts: StageChoice::Fake,
                },
                None,
            )
            .await
            .unwrap(),
        );
        let (event_tx, _rx) = broadcast::channel(64);
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
                rules: Arc::new(ArcSwap::from_pointee(RuleIndex::new())),
                dispatcher: None,
                event_bus: event_tx,
                shutdown: CancellationToken::new(),
            },
        )
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
                p.iter().any(|(t, m)| {
                    t == "assist/llm/pixel/status" && m.contains("in progress")
                })
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

        // Shapes: answers are {"text": ...}, statuses are {"status": ...}.
        let published = publisher.published.lock().unwrap().clone();
        let answer = published
            .iter()
            .find(|(t, _)| t == "assist/tts/pixel")
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&answer.1).unwrap();
        assert!(v.get("text").and_then(|t| t.as_str()).is_some_and(|s| !s.is_empty()));
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
}
```

- [ ] **Step 2: Read the fake LLM to fix the assertion**

Run: `grep -rn "impl Llm" -A 20 crates/athena-voice-providers/src/testing/`
Adjust `question_produces_status_answer_done` if the fake needs anything specific (it streams a deterministic token sequence; the generic "non-empty text" assertion above should hold as written).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p athena-voice-runtime assist::bridge 2>&1 | tail -5`
Expected: COMPILE ERROR — `AssistBridge` not defined.

- [ ] **Step 4: Implement the bridge**

`bridge.rs` (above the tests):

```rust
//! AssistBridge: consumes `{prefix}/transcription/{device}` questions and
//! answers as text on `{prefix}/tts/{device}`, with loader statuses on
//! `{prefix}/llm/{device}/status`. One actor per device, each owning a
//! router + LLM mini-pipeline (no STT/VAD/TTS actors).

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::types::Transcript;
use athena_voice_providers::ProviderFactory;

use crate::assist::topics;
use crate::intent::{IntentMatcher, RuleIndex};
use crate::pipeline::router::{RouterDeps, spawn_router};
use crate::pipeline::sentence::{IDLE_FLUSH, SentenceBuffer};
use crate::pipeline::llm::spawn_llm;
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
            if tx.try_send(text.clone()).is_ok() {
                return;
            }
            // Actor exited (idle self-reap) or is saturated with a full
            // queue of unanswered questions; drop the stale entry.
            drop(tx);
            self.devices.remove(device);
        }
        let (tx, rx) = mpsc::channel::<String>(8);
        if tx.try_send(text).is_err() {
            return; // unreachable with a fresh channel; satisfies clippy
        }
        self.devices.insert(device.to_string(), tx);
        self.spawn_device_actor(device.to_string(), rx);
    }

    fn spawn_device_actor(self: &Arc<Self>, device: String, question_rx: mpsc::Receiver<String>) {
        let bridge = self.clone();
        drop(tokio::spawn(async move {
            bridge.run_device_actor(device, question_rx).await;
        }));
    }

    async fn run_device_actor(
        self: Arc<Self>,
        device: String,
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
        // Some(deadline) while a question awaits its first answer text.
        let mut answer_deadline: Option<tokio::time::Instant> = None;
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
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    warn!(%device, "assist: no answer within timeout; releasing loader");
                    self.publish_status(&status_topic, "done").await;
                    answer_deadline = None;
                }
                ev = barge_rx.recv() => {
                    if matches!(ev, Ok(Event::BargeIn { session, .. }) if session == sid) {
                        buf.clear();
                    }
                    // Lagged/closed/other events: ignore.
                }
                () = tokio::time::sleep(IDLE_FLUSH), if !buf.is_empty() => {
                    if let Some(sentence) = buf.take() {
                        self.publish_answer(&tts_topic, &status_topic, &sentence, &mut answer_deadline).await;
                    }
                }
                maybe = question_rx.recv() => {
                    let Some(text) = maybe else { break };
                    idle_deadline = tokio::time::Instant::now() + self.init.session_idle;
                    answer_deadline = Some(tokio::time::Instant::now() + ANSWER_TIMEOUT);
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
                            self.publish_answer(&tts_topic, &status_topic, &sentence, &mut answer_deadline).await;
                        }
                        break;
                    };
                    if let Some(sentence) = buf.push(&tok) {
                        self.publish_answer(&tts_topic, &status_topic, &sentence, &mut answer_deadline).await;
                    }
                }
            }
        }

        cancel.cancel();
        self.devices.remove(&device);
        info!(%device, session = %sid, "assist: device session closed");
    }

    async fn publish_answer(
        &self,
        tts_topic: &str,
        status_topic: &str,
        sentence: &str,
        answer_deadline: &mut Option<tokio::time::Instant>,
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
        // First answer text releases the app's loader.
        if answer_deadline.take().is_some() {
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
```

Add to `assist/mod.rs`:

```rust
pub mod bridge;

pub use bridge::{AssistBridge, AssistDeps, AssistInit};
```

Note the `done`-status rule this encodes (matches the approved design): `done` fires with the FIRST sentence of each answer (loader clears as the answer bubble arrives), or after `ANSWER_TIMEOUT` if nothing ever comes. Later sentences of the same answer publish no extra statuses.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p athena-voice-runtime assist 2>&1 | tail -5`
Expected: both bridge tests + the topics tests PASS. The happy-path test exercises real router + fake LLM, so if it hangs, check the fake LLM's token output first.

- [ ] **Step 6: Full crate + lint, commit**

```bash
cargo nextest run -p athena-voice-runtime 2>&1 | tail -3
cargo fmt --all && cargo clippy -p athena-voice-runtime --all-targets -- -D warnings
git add crates/athena-voice-runtime/src/assist/
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Assist bridge: per-device text sessions over router+LLM

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Wire the bridge into `Runtime::spawn` and the MQTT loop

`Runtime::spawn` gains `assist: Option<assist::AssistInit>`. When present: build the `AssistBridge` (publisher = `AsyncClientPublisher` over the existing client), subscribe its wildcard, and hand it to the satellite loop, which routes non-satellite publishes to `bridge.handle`.

**Files:**
- Modify: `crates/athena-voice-runtime/src/lib.rs`
- Modify: `crates/athena-voice-runtime/src/satellite/ingress.rs`
- Modify: `crates/athena-voice-runtime/tests/runtime_spawn.rs`

**Interfaces:**
- Consumes: `AssistBridge::new/transcription_wildcard/handle` (Task 4).
- Produces: `Runtime::spawn(mqtt_cfg, factory, skills, assist: Option<assist::AssistInit>, session_idle)` — the signature Task 6's serve plumbing calls; `SatelliteDeps` gains `pub assist: Option<Arc<AssistBridge>>`.

- [ ] **Step 1: Extend `SatelliteDeps` and the publish dispatch**

In `satellite/ingress.rs`:

```rust
use crate::assist::AssistBridge;

pub struct SatelliteDeps {
    // ... existing fields unchanged ...
    /// Text-question bridge for the home-automation app; `None` when the
    /// `[assist]` config block is absent.
    pub assist: Option<Arc<AssistBridge>>,
    pub shutdown: CancellationToken,
}
```

In the poll loop where publishes are handled, replace the direct call with:

```rust
Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(p))) => {
    if let Some(bridge) = &deps.assist {
        if bridge.handle(&p.topic, &p.payload) {
            continue;
        }
    }
    handle_publish(&deps, &p.topic, &p.payload);
}
```

(`bridge.handle` returns false instantly for foreign topics, so satellite traffic is untouched.)

- [ ] **Step 2: Extend `Runtime::spawn`**

In `lib.rs`, change the signature and body:

```rust
pub fn spawn(
    mqtt_cfg: MqttConfig,
    factory: Arc<ProviderFactory>,
    skills: Option<SkillsInit>,
    assist: Option<assist::AssistInit>,
    session_idle: std::time::Duration,
) -> Result<Self, RuntimeError> {
```

After the skills block (so `rules`/`dispatcher` exist) and before `SatelliteDeps` is built:

```rust
let assist_bridge = assist.map(|init| {
    let bridge = assist::AssistBridge::new(
        init,
        assist::AssistDeps {
            publisher: Arc::new(wasm::host_fns::AsyncClientPublisher(client.tx.clone())),
            factory: factory.clone(),
            matcher: matcher.clone(),
            rules: rules.clone(),
            dispatcher: dispatcher.clone(),
            event_bus: event_bus.sender(),
            shutdown: shutdown.clone(),
        },
    );
    let wildcard = bridge.transcription_wildcard();
    let mqtt = client.tx.clone();
    drop(tokio::spawn(async move {
        // Queued like the satellite subscribe; rumqttc retries on reconnect.
        if let Err(e) = mqtt.subscribe(wildcard, rumqttc::QoS::AtMostOnce).await {
            tracing::warn!(error = %e, "assist subscribe failed");
        }
    }));
    bridge
});
```

and pass `assist: assist_bridge,` in `SatelliteDeps`. (QoS 0 matches what the app's gateway uses.) Note: `matcher`, `rules`, and `dispatcher` are moved into `SatelliteDeps` today — clone them for the bridge BEFORE that struct is built.

- [ ] **Step 3: Fix the two existing call sites**

- `crates/athena-voice-runtime/tests/runtime_spawn.rs`: add `None,` as the new 4th argument.
- `crates/athena-voice-cli/src/serve.rs`: add `None,` for now (Task 6 replaces it with real plumbing).

- [ ] **Step 4: Add a spawn-with-assist smoke test**

Append to `tests/runtime_spawn.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_with_assist_bridge_is_clean() {
    let factory = Arc::new(
        ProviderFactory::build(
            &ProviderConfig {
                stt: StageChoice::Fake,
                llm: StageChoice::Fake,
                tts: StageChoice::Fake,
            },
            None,
        )
        .await
        .unwrap(),
    );
    let runtime = Runtime::spawn(
        MqttConfig {
            host: "127.0.0.1".into(),
            port: 62991,
            client_id: "athena-voice-assist-test".into(),
            username: None,
            password: None,
            keep_alive_secs: 30,
        },
        factory,
        None,
        Some(athena_voice_runtime::assist::AssistInit {
            topic_prefix: "assist".into(),
            locale: athena_voice_core::ids::Locale::new("fr").unwrap(),
            session_idle: std::time::Duration::from_secs(120),
        }),
        std::time::Duration::from_secs(120),
    )
    .expect("spawn with assist");
    runtime.shutdown.cancel();
}
```

- [ ] **Step 5: Run, lint, commit**

Run: `cargo nextest run -p athena-voice-runtime 2>&1 | tail -3` then `cargo check --workspace --all-targets 2>&1 | tail -2`
Expected: all green, workspace compiles (serve.rs updated).

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/athena-voice-runtime/ crates/athena-voice-cli/src/serve.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Runtime: wire the assist bridge into spawn and the MQTT loop

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: `[assist]` config, serve plumbing, and the GEEKOM profile

**Files:**
- Modify: `crates/athena-voice-cli/src/config.rs`
- Modify: `crates/athena-voice-cli/src/serve.rs`
- Create: `athena.assist.toml`

**Interfaces:**
- Consumes: `Runtime::spawn(..., assist, ...)` (Task 5), `assist::AssistInit` (Task 4).
- Produces: `Config.assist: Option<AssistConfig>` with `pub struct AssistConfig { pub enabled: bool, pub topic_prefix: String, pub locale: Locale }`.

- [ ] **Step 1: Write the failing config test**

Append to the tests module in `crates/athena-voice-cli/src/config.rs`:

```rust
#[test]
fn parses_assist_block_and_defaults() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        tmp.path(),
        r#"
locales = ["fr"]
[server]
host = "0.0.0.0"
port = 8080
[storage]
database_url = "sqlite::memory:"
[mqtt]
host = "127.0.0.1"
port = 1883
client_id = "test"
[providers]
stt = "fake"
llm = "fake"
tts = "fake"
[assist]
enabled = true
"#,
    )
    .unwrap();
    let cfg = load(tmp.path()).unwrap();
    let assist = cfg.assist.expect("assist block parsed");
    assert!(assist.enabled);
    assert_eq!(assist.topic_prefix, "assist");
    assert_eq!(assist.locale.as_str(), "fr");
}

#[test]
fn missing_assist_block_is_none() {
    // athena.example.toml has no [assist] block.
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let cfg = load(&repo_root.join("athena.example.toml")).expect("example parses");
    assert!(cfg.assist.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p athena-voice-cli config 2>&1 | tail -5`
Expected: COMPILE ERROR — no `assist` field.

- [ ] **Step 3: Implement `AssistConfig`**

In `config.rs`:

```rust
/// `[assist]` section: text bridge for the owner's home-automation app.
/// Absent block = bridge off; `enabled = false` also turns it off without
/// deleting the section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssistConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_assist_prefix")]
    pub topic_prefix: String,
    #[serde(default = "default_assist_locale")]
    pub locale: Locale,
}

fn default_true() -> bool {
    true
}

fn default_assist_prefix() -> String {
    "assist".into()
}

fn default_assist_locale() -> Locale {
    Locale::new("fr").expect("static locale")
}
```

and in `Config`: `#[serde(default)] pub assist: Option<AssistConfig>,`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p athena-voice-cli 2>&1 | tail -3`
Expected: PASS (both new tests + existing).

- [ ] **Step 5: Plumb into serve**

In `serve.rs`, replace the `None,` placeholder from Task 5:

```rust
let assist = cfg.assist.as_ref().filter(|a| a.enabled).map(|a| {
    athena_voice_runtime::assist::AssistInit {
        topic_prefix: a.topic_prefix.clone(),
        locale: a.locale.clone(),
        session_idle: std::time::Duration::from_secs(cfg.server.session_idle_secs),
    }
});
```

and pass `assist,` to `Runtime::spawn`. Add a startup log so the profile is visible:

```rust
if let Some(a) = cfg.assist.as_ref().filter(|a| a.enabled) {
    tracing::info!(prefix = %a.topic_prefix, locale = %a.locale.as_str(), "assist bridge enabled");
}
```

- [ ] **Step 6: Create `athena.assist.toml`**

```toml
# Athena-Voice — GEEKOM / home-server profile.
#
# One process, no audio: questions arrive as TEXT from the DomoticApp
# (on-device speech recognition) via the LAN MQTT broker, answers go back
# as TEXT and are spoken by the phone. No STT/TTS worker needed; the
# "fake" providers below are never invoked for assist sessions.
#
# Fill in [mqtt] host with your broker's LAN address. If the broker needs
# credentials, set username here and pass the password via the environment:
#   ATHENA__MQTT__PASSWORD=... athena-voice-cli serve --config athena.assist.toml

locales = ["fr", "en"]

[server]
host = "0.0.0.0"          # admin web UI
port = 8080
session_idle_secs = 120

[storage]
database_url = "sqlite://athena-voice.db?mode=rwc"

[mqtt]
host = "192.168.1.2"      # <-- your LAN broker
port = 1883
client_id = "athena-voice"
# username = "athena"

[providers]
stt = "fake"              # unused by the assist bridge
llm = "none"              # unmatched questions get a spoken capabilities answer
tts = "fake"              # unused by the assist bridge

[assist]
enabled = true
topic_prefix = "assist"
locale = "fr"

[skills]
dir = "./skills"

[skills.weather]
http_allowlist = ["geocoding-api.open-meteo.com", "api.open-meteo.com"]

[skills.jeedom]
# Configure base_url/api_key via the admin web UI (stored in SQLite, not here).
http_allowlist = []
```

Check the weather skill's real allowlist hosts against `athena.voice.toml` and copy them verbatim rather than trusting the above.

- [ ] **Step 7: Full workspace check, lint, commit**

Run: `cargo nextest run --workspace 2>&1 | tail -3`

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/athena-voice-cli/ athena.assist.toml
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "CLI: [assist] config block and the GEEKOM assist profile

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: Integration test — real skill answers over the bridge

Proves the full assist path with a REAL WASM skill (the smoke-test skill answers time queries in FR/EN), plus barge-in. Follows the harness pattern of `tests/en_end_to_end.rs`: registry + dispatcher + matcher wired manually, no broker.

**Files:**
- Create: `crates/athena-voice-runtime/tests/assist_end_to_end.rs`

**Interfaces:**
- Consumes: `AssistBridge`/`AssistDeps`/`AssistInit`, `test_support::SMOKE_TEST_WASM`, registry/dispatcher setup copied from `tests/en_end_to_end.rs`.

- [ ] **Step 1: Copy the harness scaffolding**

Open `crates/athena-voice-runtime/tests/en_end_to_end.rs` and copy its skill-loading preamble (store, bogus-broker `MqttClient`, `SkillCtx`, `host_functions`, `ExtismSkillPlugin`, `SkillRegistry`, `SkillDispatcher::spawn`, `IntentMatcher` + `registry.patterns_handle()`) into the new file, keeping `SKILL_NAME = "smoke-test"`. Reuse the `RecordingPublisher` idea from Task 4's unit tests — duplicate it here (test files can't import each other's test modules; it's 30 lines).

- [ ] **Step 2: Write the two tests**

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn french_time_question_answered_as_text() {
    // ... scaffolding from step 1: registry, dispatcher, matcher, rules ...
    let publisher = RecordingPublisher::new();
    let bridge = AssistBridge::new(
        AssistInit {
            topic_prefix: "assist".into(),
            locale: Locale::new("fr").unwrap(),
            session_idle: Duration::from_secs(120),
        },
        AssistDeps {
            publisher: publisher.clone(),
            factory, // StageChoice::None for llm — skills answer, LLM must not exist
            matcher,
            rules,
            dispatcher: Some(dispatcher_handle),
            event_bus: event_tx,
            shutdown: CancellationToken::new(),
        },
    );

    assert!(bridge.handle(
        "assist/transcription/pixel",
        br#"{"text": "quelle heure est-il"}"#
    ));

    publisher
        .wait_for(|p| p.iter().any(|(t, _)| t == "assist/tts/pixel"))
        .await;
    let published = publisher.published.lock().unwrap().clone();
    let (_, answer) = published
        .iter()
        .find(|(t, _)| t == "assist/tts/pixel")
        .unwrap()
        .clone();
    let v: serde_json::Value = serde_json::from_str(&answer).unwrap();
    let text = v["text"].as_str().unwrap();
    // The smoke skill speaks the actual local time in French.
    assert!(text.contains("heure") || text.contains(':') || text.contains('h'),
        "unexpected answer: {text}");
    // Loader lifecycle: in progress before the answer, done at/after it.
    let idx = |pred: &dyn Fn(&(String, String)) -> bool| published.iter().position(|x| pred(x));
    let in_progress = idx(&|(t, m)| t == "assist/llm/pixel/status" && m.contains("in progress")).expect("in-progress status");
    let answer_idx = idx(&|(t, _)| t == "assist/tts/pixel").unwrap();
    let done = idx(&|(t, m)| t == "assist/llm/pixel/status" && m.contains("done")).expect("done status");
    assert!(in_progress < answer_idx && answer_idx <= done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_question_supersedes_first() {
    // Same scaffolding. Send two questions back-to-back; assert the actor
    // stays alive and BOTH produce answers eventually (barge-in cancels
    // in-flight work, then the second runs) — and no panic/hang occurs.
    // Assert at least one answer arrives and the LAST answer corresponds
    // to a live pipeline (publish count keeps growing until quiescent).
}
```

Fill the second test concretely: send `{"text": "quelle heure est-il"}` twice with no delay, `wait_for` at least one `assist/tts/pixel` message, then sleep 1 s and assert no further growth (quiescence) and that a `done` status exists.

Check the smoke skill's actual FR answer text in `skills-smoke-test/src/` FIRST and tighten the answer assertion to something it really produces (e.g. it contains "il est").

- [ ] **Step 3: Run to verify the tests fail meaningfully, then pass**

Run: `cargo nextest run -p athena-voice-runtime --test assist_end_to_end 2>&1 | tail -5`
First run of a NEW test against already-implemented code should pass immediately — that's expected here (this is an integration pin, not TDD of new behavior; the TDD happened in Task 4). If it fails, the bridge has a real wiring bug: debug it, don't weaken the test.

- [ ] **Step 4: Lint and commit**

```bash
cargo fmt --all && cargo clippy -p athena-voice-runtime --all-targets -- -D warnings
git add crates/athena-voice-runtime/tests/assist_end_to_end.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Integration test: assist bridge answers via the smoke skill

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Docs, live verification, PLAN.md, ship

**Files:**
- Modify: `README.md`
- Modify: `PLAN.md`

- [ ] **Step 1: README — "Run on a Linux box (GEEKOM)" section**

Add after the existing modes/setup content:

```markdown
## Run on a Linux box (GEEKOM / home server)

The assist profile answers **text** questions from a home-automation app
over MQTT — no audio stack, no whisper, no TTS engine on the server.

    # once: Rust + a C toolchain
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    sudo apt install build-essential pkg-config

    git clone https://github.com/Gekkotron/Athena-Voice.git && cd Athena-Voice
    for s in skills-smoke-test skills-weather skills-timer skills-home skills-jeedom; do ./$s/build.sh; done
    mkdir -p skills && cp skills-*/target/wasm32-wasip1/release/*.wasm skills/

    # point [mqtt] host at your LAN broker, then:
    cargo run --release -p athena-voice-cli -- serve --config athena.assist.toml

Broker credentials never live in the TOML: set `username` there and pass
`ATHENA__MQTT__PASSWORD=...` in the environment (any `[mqtt]` field can be
overridden as `ATHENA__MQTT__<FIELD>`).

### Talking to it (DomoticApp protocol)

| Topic | Direction | Payload |
| --- | --- | --- |
| `assist/transcription/{device}` | app → Athena | `{"text": "quelle heure est-il"}` |
| `assist/tts/{device}` | Athena → app | `{"text": "il est 15 h 14"}` (one message per sentence) |
| `assist/llm/{device}/status` | Athena → app | `{"status": "in progress"}` then `{"status": "done"}` |

Try it without the app:

    mosquitto_sub -h <broker> -t 'assist/tts/#' -v &
    mosquitto_pub -h <broker> -t assist/transcription/cli \
      -m '{"text": "quelle heure est-il"}'

### Keep it running (systemd)

    # /etc/systemd/system/athena-voice.service
    [Unit]
    Description=Athena-Voice assist bridge
    After=network-online.target

    [Service]
    WorkingDirectory=/home/<you>/Athena-Voice
    Environment=ATHENA__MQTT__PASSWORD=<secret>   # or use an EnvironmentFile
    ExecStart=/home/<you>/Athena-Voice/target/release/athena-voice-cli serve --config athena.assist.toml
    Restart=on-failure

    [Install]
    WantedBy=multi-user.target
```

Verify the skill build commands against the real `skills-*/build.sh` outputs (artifact paths) and the real CLI binary name in `crates/athena-voice-cli/Cargo.toml` (`[[bin]]` name) — fix the README text to match reality, not the other way around.

- [ ] **Step 2: Live verification on this Mac (plan ground rule)**

```bash
# broker
mosquitto -p 1884 -v &
# profile copy pointing at localhost:1884, fake skills dir with built wasm
cargo run --release -p athena-voice-cli -- serve --config /tmp/athena.assist.local.toml &
mosquitto_sub -h 127.0.0.1 -p 1884 -t 'assist/#' -v &
mosquitto_pub -h 127.0.0.1 -p 1884 -t assist/transcription/cli -m '{"text": "quelle heure est-il"}'
```

Expected in the sub window: `assist/llm/cli/status {"status": "in progress"}`, then `assist/tts/cli {"text": "il est ..."}`, then `{"status": "done"}`. Also verify the malformed case (`-m 'garbage'` → nothing published) and idle reap after 120 s (log line "assist: device session idle"). Kill all three processes afterwards. If mosquitto is not installed: `brew install mosquitto`.

- [ ] **Step 3: Run the full gate**

```bash
cargo check --workspace --all-targets && cargo nextest run --workspace && ./SHOWCASE.sh
```

Expected: everything green.

- [ ] **Step 4: PLAN.md Done entry + commit + push**

Add under `## Done` (respecting the PLAN.md format contract — no blank lines inside the body):

```markdown
- [x] Assist text bridge + GEEKOM Linux profile (2026-07-31)
      New `[assist]` block: runtime subscribes `assist/transcription/+` on the LAN broker, routes text through the normal intent/skill/LLM pipeline per device, and answers as `{"text": …}` on `assist/tts/{device}` with loader statuses on `assist/llm/{device}/status` — DomoticApp protocol, zero app changes. SentenceBuffer extracted from the TTS actor and shared. `athena.assist.toml` profile (no audio providers used), README Linux run-book + systemd unit, ci.yml YAML repaired (Linux CI green again). Live-verified with mosquitto_pub/sub. Spec: docs/superpowers/specs/2026-07-31-assist-bridge-geekom-design.md.
```

```bash
git add README.md PLAN.md
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "README: Linux/GEEKOM run-book for the assist bridge; PLAN update

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
gh run list --repo Gekkotron/Athena-Voice --limit 1
```

Expected: push succeeds; CI run starts (and the `test` job passes on ubuntu-latest — the Linux proof for everything above).

---

## Self-Review (done at authoring time)

- **Spec coverage:** ingress/validation → T3+T4; sessions/answer path/statuses → T4; barge-in → T4/T7; config + profile → T6; credentials → docs only (spec correction 1) → T8; Linux CI → T1 (spec correction 2); README/systemd → T8; integration + live tests → T7/T8; fail-fast broker/password items are inherited from existing serve behavior (rumqttc connect error surfaces at spawn; figment env override) — no new code.
- **Type consistency:** `AssistInit`/`AssistDeps`/`AssistBridge` names and the `spawn(... assist ...)` arg order match across T4/T5/T6; `SentenceBuffer` API in T2 matches T4's usage.
- **Placeholder scan:** the two "check the real code first" notes (fake-LLM output, smoke-skill FR text, build.sh artifact paths) are deliberate verification instructions, not gaps — each names the exact file to read.
