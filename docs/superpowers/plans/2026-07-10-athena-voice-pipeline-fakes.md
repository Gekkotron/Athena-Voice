# Athena-Voice — Plan 2: Voice Pipeline (Fakes End-to-End)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the full voice pipeline end-to-end using fake providers. After this plan, an in-process integration test can: (1) start an embedded MQTT broker, (2) drive a fake satellite that publishes `session/start` + PCM audio chunks + `session/end`, (3) watch the pipeline transcribe (via `FakeStt`), route to LLM fallback (no skills yet), synthesise (via `FakeTts`), publish TTS chunks back on the satellite egress topics, and emit the golden `Event` sequence. `athena-voice serve` (without `--dry-run`) will boot the entire runtime, connect to a real mosquitto and process real MQTT satellite traffic.

**Architecture:** Two new crates — `athena-voice-providers` (fake `Stt`/`Llm`/`Tts` impls behind a `factory`) and `athena-voice-runtime` (actor DAG: `SatelliteAdapter` → `Ingest` → `Vad` → `Stt` → `IntentRouter` → `Llm` fallback → `Tts` → `ResponseSink`, all tokio tasks with bounded `mpsc` channels). MQTT via `rumqttc`; session state in a `DashMap<SessionId, SessionState>` guarded by per-session `CancellationToken`. Event bus is a `tokio::sync::broadcast::Sender<Event>` mirrored to `athena/events/*` on MQTT by a separate task. `athena-voice-cli::serve::run` gains a real body: build the actor DAG, block on SIGTERM, drain gracefully.

**Tech Stack:** `rumqttc` 0.24 (MQTT v5 client), `rumqttd` 0.19 (embedded broker for tests only, dev-dep), `dashmap` 6, `tokio-util` 0.7 (CancellationToken), `futures` 0.3, `serial_test` 3 (env-touching test isolation). All existing Plan 1 crates unchanged in public API — only additions.

## Global Constraints

- **No real ML providers.** `Stt`/`Llm`/`Tts` impls in this plan are exclusively `Fake*` deterministic implementations. Real providers land in Plan 3.
- **No skill dispatcher yet.** `IntentRouter` always falls through to LLM in Plan 2. Skill dispatch and pattern matching land in Plan 4.
- **No dashboard.** `Event` broadcast + MQTT mirror land here, but the axum HTTP dashboard consuming them lands in Plan 5.
- **MQTT topic tree** frozen per spec §6.3 — do not invent new topic paths.
- **Session-scoped everything.** Per-session `CancellationToken`, per-session `mpsc<AudioFrame>` (cap 64), per-session `mpsc<Transcript>` (cap 16). Actors that fan out beyond one session use a shared spawned handle plus request-scoped correlation via `SessionId`.
- **QoS values fixed** per spec §6.3. Do not change without spec revision.
- **Every actor’s future selects on its `CancellationToken` first.** Drop the token → aborts within one tick.
- **All new tests use `tokio::time::pause()` where time is involved.** No real `sleep()` in tests.
- **Every failing-path assertion has a matching integration test** (per spec §8.4 — Plan 2 covers the subset of the failure matrix reachable without real providers).
- **Naming:** everything under `athena-voice-*`. Attribution follows the global rule.
- **No pushing** between commits unless the executor explicitly does `git push` at the end.
- Rust edition/toolchain from Plan 1 (`edition = "2024"`, `channel = "1.88"`).

---

## File structure produced by this plan

```
athena-voice/
├── Cargo.toml                                           # (mod) workspace: add 2 members + shared deps
├── locales/
│   ├── fr.toml                                          # (new) FR canned prompts + error phrases
│   └── en.toml                                          # (new) EN counterpart
├── crates/
│   ├── athena-voice-core/                               # (unchanged)
│   ├── athena-voice-storage/                            # (unchanged)
│   ├── athena-voice-providers/                          # (new)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                                   # re-exports
│   │       ├── factory.rs                               # ProviderFactory + config-driven picks
│   │       ├── error.rs                                 # SttError / LlmError / TtsError
│   │       └── testing/
│   │           ├── mod.rs
│   │           ├── fake_stt.rs                          # FakeStt: preset transcripts
│   │           ├── fake_llm.rs                          # FakeLlm: echo + rule map
│   │           └── fake_tts.rs                          # FakeTts: silent Opus-like frames
│   ├── athena-voice-runtime/                            # (new) the actor DAG
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                                   # Runtime::spawn public entry
│   │       ├── config.rs                                # RuntimeConfig (subset consumed by runtime)
│   │       ├── error.rs                                 # RuntimeError
│   │       ├── event_bus.rs                             # broadcast::Sender<Event> + MQTT mirror
│   │       ├── locale.rs                                # LocalePack + loader (locales/*.toml)
│   │       ├── mqtt/
│   │       │   ├── mod.rs                               # MqttClient wrapper
│   │       │   ├── topics.rs                            # topic constants + parser
│   │       │   └── publish.rs                           # typed publish helpers
│   │       ├── session/
│   │       │   ├── mod.rs                               # SessionState + registry
│   │       │   └── manager.rs                           # SessionManager (DashMap+tokens)
│   │       ├── satellite/
│   │       │   ├── mod.rs                               # SatelliteAdapter actor
│   │       │   ├── ingress.rs                           # MQTT → AudioFrame stream
│   │       │   └── egress.rs                            # Event → MQTT publish
│   │       └── pipeline/
│   │           ├── mod.rs                               # spawn all actors and wire channels
│   │           ├── ingest.rs                            # Ingest actor (frame passthrough + check)
│   │           ├── vad.rs                               # Vad actor (endpoint marker)
│   │           ├── stt.rs                               # Stt actor (wraps provider)
│   │           ├── router.rs                            # IntentRouter (Plan 2: always LLM)
│   │           ├── llm.rs                               # Llm actor
│   │           ├── tts.rs                               # Tts actor
│   │           └── sink.rs                              # ResponseSink (MQTT publish adapter)
│   └── athena-voice-cli/                                # (mod)
│       ├── Cargo.toml                                   # (mod) add runtime + providers deps
│       └── src/
│           ├── config.rs                                # (mod) add mqtt + providers sections
│           └── serve.rs                                 # (mod) build actor DAG, block on SIGTERM
```

---

## Task 1: Workspace + crate stubs

**Files:**
- Modify: `Cargo.toml` (workspace members + shared deps)
- Create: `crates/athena-voice-providers/Cargo.toml`
- Create: `crates/athena-voice-providers/src/lib.rs`
- Create: `crates/athena-voice-runtime/Cargo.toml`
- Create: `crates/athena-voice-runtime/src/lib.rs`

**Interfaces:**
- Consumes: `athena-voice-core` traits and types (Plan 1).
- Produces: two stub crates registered in the workspace; `cargo check --workspace` clean.

- [ ] **Step 1: Add workspace deps to root `Cargo.toml`**

Add to `[workspace.dependencies]`:

```toml
dashmap = "6"
futures = "0.3"
rumqttc = { version = "0.24", default-features = false, features = ["use-native-tls"] }
```

Add to `[workspace]`:

```toml
members = [
    "crates/athena-voice-core",
    "crates/athena-voice-storage",
    "crates/athena-voice-providers",
    "crates/athena-voice-runtime",
    "crates/athena-voice-cli",
]
```

- [ ] **Step 2: Create `crates/athena-voice-providers/Cargo.toml`**

```toml
[package]
name = "athena-voice-providers"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "STT / LLM / TTS provider adapters for Athena-Voice (Plan 2: fakes only)."

[dependencies]
async-trait = { workspace = true }
bytes = { workspace = true }
futures = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

athena-voice-core = { path = "../athena-voice-core", version = "0.1" }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread"] }

[lints]
workspace = true
```

- [ ] **Step 3: Create `crates/athena-voice-providers/src/lib.rs`**

```rust
#![deny(warnings)]
//! Provider adapters for Athena-Voice. Plan 2 ships fakes only; real providers land in Plan 3.

pub mod error;
pub mod factory;
pub mod testing;

pub use error::{LlmError, SttError, TtsError};
```

- [ ] **Step 4: Create empty `crates/athena-voice-providers/src/{error,factory}.rs` and `testing/mod.rs`**

`error.rs`:
```rust
//! Placeholder; fills in Task 2.
```

`factory.rs`:
```rust
//! Placeholder; fills in Task 6.
```

`testing/mod.rs`:
```rust
//! Deterministic fake providers used by integration tests and Plan 2 default runtime.
```

- [ ] **Step 5: Create `crates/athena-voice-runtime/Cargo.toml`**

```toml
[package]
name = "athena-voice-runtime"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Actor DAG, MQTT client, satellite adapter, and event bus for Athena-Voice."

[dependencies]
async-trait = { workspace = true }
bytes = { workspace = true }
chrono = { workspace = true }
dashmap = { workspace = true }
futures = { workspace = true }
rumqttc = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tokio-util = { workspace = true }
tracing = { workspace = true }
toml = "0.8"

athena-voice-core = { path = "../athena-voice-core", version = "0.1" }
athena-voice-providers = { path = "../athena-voice-providers", version = "0.1" }
athena-voice-storage = { path = "../athena-voice-storage", version = "0.1" }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread", "test-util"] }
rumqttd = { version = "0.19", default-features = false }
tempfile = "3"

[lints]
workspace = true
```

- [ ] **Step 6: Create `crates/athena-voice-runtime/src/lib.rs`**

```rust
#![deny(warnings)]
//! Athena-Voice runtime: actor DAG + MQTT satellite adapter + event bus.

pub mod config;
pub mod error;
pub mod event_bus;
pub mod locale;
pub mod mqtt;
pub mod pipeline;
pub mod satellite;
pub mod session;

pub use error::RuntimeError;
```

Each `pub mod` gets an empty stub file for now (`config.rs`, `error.rs`, `event_bus.rs`, `locale.rs`, `mqtt/mod.rs`, `pipeline/mod.rs`, `satellite/mod.rs`, `session/mod.rs`). Task 1 just wires them up.

For each stub file write:
```rust
//! Placeholder; fills in later tasks.
```

- [ ] **Step 7: Run `cargo check --workspace`**

Run: `cargo check --workspace`
Expected: two new crates compile as empty modules; workspace clean.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/athena-voice-providers crates/athena-voice-runtime
git commit -m "chore(runtime,providers): stub crates registered in workspace"
```

---

## Task 2: Provider errors

**Files:**
- Modify: `crates/athena-voice-providers/src/error.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub enum SttError`, `LlmError`, `TtsError` (typed via `thiserror`), each with `is_retryable(&self) -> bool` and `variant_name(&self) -> &'static str` (used by storage `append_error`).

- [ ] **Step 1: Write failing tests inside `error.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_is_retryable_truth_table() {
        assert!(SttError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(SttError::Unavailable { name: "fake", source: "boom".into() }.is_retryable());
        assert!(!SttError::BadAudio("bad".into()).is_retryable());
        assert!(!SttError::CircuitOpen { retry_after_ms: 60_000 }.is_retryable());
        assert!(!SttError::Cancelled.is_retryable());
    }

    #[test]
    fn llm_no_retry_by_default() {
        assert!(!LlmError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(!LlmError::Unavailable { name: "fake", source: "boom".into() }.is_retryable());
    }

    #[test]
    fn tts_retryable_on_transient() {
        assert!(TtsError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(TtsError::Unavailable { name: "fake", source: "boom".into() }.is_retryable());
        assert!(!TtsError::Cancelled.is_retryable());
    }

    #[test]
    fn variant_names_are_stable_strings() {
        assert_eq!(SttError::Timeout { name: "x", ms: 0 }.variant_name(), "Timeout");
        assert_eq!(LlmError::Cancelled.variant_name(), "Cancelled");
        assert_eq!(TtsError::BadAudio("".into()).variant_name(), "BadAudio");
    }
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-providers --lib error::tests`
Expected: compile error, types absent.

- [ ] **Step 3: Implement `error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("stt provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("stt provider {name} unavailable: {source}")]
    Unavailable { name: &'static str, source: String },
    #[error("bad audio: {0}")]
    BadAudio(String),
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl SttError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::Unavailable { .. })
    }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::BadAudio(_) => "BadAudio",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("llm provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("llm provider {name} unavailable: {source}")]
    Unavailable { name: &'static str, source: String },
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    /// LLM is nondeterministic — retrying can produce a different answer. Never auto-retry.
    #[must_use]
    pub fn is_retryable(&self) -> bool { false }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("tts provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("tts provider {name} unavailable: {source}")]
    Unavailable { name: &'static str, source: String },
    #[error("bad audio: {0}")]
    BadAudio(String),
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl TtsError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::Unavailable { .. })
    }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::BadAudio(_) => "BadAudio",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p athena-voice-providers --lib error::tests`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-providers
git commit -m "feat(providers): typed SttError/LlmError/TtsError with is_retryable"
```

---

## Task 3: FakeStt

**Files:**
- Create: `crates/athena-voice-providers/src/testing/fake_stt.rs`
- Modify: `crates/athena-voice-providers/src/testing/mod.rs`

**Interfaces:**
- Consumes: `Stt` trait, `Transcript`, `Locale`, `SessionId`, `AudioFrame` (core).
- Produces: `pub struct FakeStt` implementing `Stt`. Behaviour:
    - Accepts a preset `HashMap<SessionId, Vec<Transcript>>` (typically one partial + one final).
    - `transcribe` returns a stream that emits the preset transcripts and completes.
    - Preset lookup: (a) exact `SessionId`, else (b) session-agnostic fallback via `default_transcripts`.
    - `name() -> "fake-stt"`.

- [ ] **Step 1: Register the module**

`testing/mod.rs`:
```rust
//! Deterministic fake providers used by integration tests and the Plan 2 default runtime.

pub mod fake_stt;
```

- [ ] **Step 2: Write failing tests inside `fake_stt.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::{Locale, SessionId};
    use athena_voice_core::provider::Stt;
    use athena_voice_core::types::Transcript;
    use futures::stream::{self, StreamExt};

    fn transcript(text: &str, is_final: bool) -> Transcript {
        Transcript { text: text.into(), is_final, confidence: Some(1.0) }
    }

    #[tokio::test]
    async fn emits_preset_transcripts_in_order() {
        let sid = SessionId::new_v4();
        let stt = FakeStt::builder()
            .preset(sid, vec![transcript("bon", false), transcript("bonjour", true)])
            .build();

        let audio: Box<dyn futures::Stream<Item = _> + Send + Unpin> =
            Box::new(stream::empty());
        let audio = Box::pin(audio) as _;

        let mut ts = stt.transcribe(sid, Locale::new("fr").unwrap(), audio).await.unwrap();
        let a = ts.next().await.unwrap().unwrap();
        let b = ts.next().await.unwrap().unwrap();
        assert!(ts.next().await.is_none());

        assert_eq!(a.text, "bon");
        assert!(!a.is_final);
        assert_eq!(b.text, "bonjour");
        assert!(b.is_final);
    }

    #[tokio::test]
    async fn falls_back_to_default_when_no_preset() {
        let stt = FakeStt::builder()
            .default_transcripts(vec![transcript("hello", true)])
            .build();

        let sid = SessionId::new_v4();
        let audio = Box::pin(stream::empty());
        let mut ts = stt.transcribe(sid, Locale::new("en").unwrap(), audio).await.unwrap();
        let final_t = ts.next().await.unwrap().unwrap();
        assert_eq!(final_t.text, "hello");
        assert!(final_t.is_final);
    }

    #[tokio::test]
    async fn empty_default_yields_empty_stream() {
        let stt = FakeStt::builder().build();
        let sid = SessionId::new_v4();
        let audio = Box::pin(stream::empty());
        let mut ts = stt.transcribe(sid, Locale::new("en").unwrap(), audio).await.unwrap();
        assert!(ts.next().await.is_none());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FakeStt::builder().build().name(), "fake-stt");
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-providers --lib testing::fake_stt::tests`
Expected: `FakeStt` unresolved.

- [ ] **Step 4: Implement `fake_stt.rs`**

```rust
use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioFrameStream, BoxError, Stt, TranscriptStream};
use athena_voice_core::types::Transcript;

pub struct FakeStt {
    preset: HashMap<SessionId, Vec<Transcript>>,
    default: Vec<Transcript>,
}

impl FakeStt {
    #[must_use]
    pub fn builder() -> FakeSttBuilder {
        FakeSttBuilder::default()
    }
}

#[derive(Default)]
pub struct FakeSttBuilder {
    preset: HashMap<SessionId, Vec<Transcript>>,
    default: Vec<Transcript>,
}

impl FakeSttBuilder {
    #[must_use]
    pub fn preset(mut self, session: SessionId, transcripts: Vec<Transcript>) -> Self {
        self.preset.insert(session, transcripts);
        self
    }

    #[must_use]
    pub fn default_transcripts(mut self, transcripts: Vec<Transcript>) -> Self {
        self.default = transcripts;
        self
    }

    #[must_use]
    pub fn build(self) -> FakeStt {
        FakeStt { preset: self.preset, default: self.default }
    }
}

#[async_trait]
impl Stt for FakeStt {
    async fn transcribe(
        &self,
        session: SessionId,
        _locale: Locale,
        // We accept the audio stream to satisfy the trait but consume nothing —
        // the fake is entirely time-agnostic and returns preset transcripts.
        _audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError> {
        let items = self
            .preset
            .get(&session)
            .cloned()
            .unwrap_or_else(|| self.default.clone());
        let s = stream::iter(items.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
    }

    fn name(&self) -> &'static str {
        "fake-stt"
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-providers --lib testing::fake_stt::tests`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-providers
git commit -m "feat(providers): FakeStt with preset + default transcripts"
```

---

## Task 4: FakeLlm

**Files:**
- Create: `crates/athena-voice-providers/src/testing/fake_llm.rs`
- Modify: `crates/athena-voice-providers/src/testing/mod.rs`

**Interfaces:**
- Consumes: `Llm` trait, `Locale`, `SessionId`, `Completion` (unused here — this fake emits tokens directly).
- Produces: `pub struct FakeLlm` implementing `Llm`. Behaviour:
    - Accepts a `rules: Vec<(String, String)>` (prompt-substring → response).
    - `complete(prompt, ...)` returns a stream that first emits the response as individual space-delimited tokens (one per stream item), then completes.
    - Unmatched prompts return `"je ne sais pas"` for `fr` locale, `"i don't know"` for others.
    - Optional `delay_ms` per token (for testing streaming latency; default 0).
    - `name() -> "fake-llm"`.

- [ ] **Step 1: Register module**

Append to `testing/mod.rs`:
```rust
pub mod fake_llm;
```

- [ ] **Step 2: Write failing tests in `fake_llm.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::{Locale, SessionId};
    use athena_voice_core::provider::Llm;
    use futures::stream::StreamExt;

    #[tokio::test]
    async fn matched_rule_streams_tokens() {
        let llm = FakeLlm::builder()
            .rule("weather", "il fait beau")
            .build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("fr").unwrap(),
                "quel est le weather".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(t) = tokens.next().await {
            out.push(t.unwrap());
        }
        assert_eq!(out, vec!["il", "fait", "beau"]);
    }

    #[tokio::test]
    async fn unmatched_fr_returns_je_ne_sais_pas() {
        let llm = FakeLlm::builder().build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("fr").unwrap(),
                "quel est le sens de la vie".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut joined = String::new();
        while let Some(t) = tokens.next().await {
            joined.push_str(&t.unwrap());
            joined.push(' ');
        }
        assert!(joined.contains("je ne sais pas"));
    }

    #[tokio::test]
    async fn unmatched_en_returns_i_dont_know() {
        let llm = FakeLlm::builder().build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "meaning of life".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut joined = String::new();
        while let Some(t) = tokens.next().await {
            joined.push_str(&t.unwrap());
            joined.push(' ');
        }
        assert!(joined.contains("don't know") || joined.contains("dont know"));
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-providers --lib testing::fake_llm::tests`

- [ ] **Step 4: Implement `fake_llm.rs`**

```rust
use async_trait::async_trait;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, CompletionStream, Llm};

pub struct FakeLlm {
    rules: Vec<(String, String)>,
}

impl FakeLlm {
    #[must_use]
    pub fn builder() -> FakeLlmBuilder {
        FakeLlmBuilder::default()
    }
}

#[derive(Default)]
pub struct FakeLlmBuilder {
    rules: Vec<(String, String)>,
}

impl FakeLlmBuilder {
    #[must_use]
    pub fn rule(mut self, prompt_substr: impl Into<String>, response: impl Into<String>) -> Self {
        self.rules.push((prompt_substr.into(), response.into()));
        self
    }

    #[must_use]
    pub fn build(self) -> FakeLlm {
        FakeLlm { rules: self.rules }
    }
}

fn fallback(locale: &Locale) -> &'static str {
    if locale.as_str().starts_with("fr") {
        "je ne sais pas"
    } else {
        "i don't know"
    }
}

#[async_trait]
impl Llm for FakeLlm {
    async fn complete(
        &self,
        _session: SessionId,
        locale: Locale,
        prompt: String,
        _history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        let response = self
            .rules
            .iter()
            .find(|(sub, _)| prompt.contains(sub))
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| fallback(&locale).to_string());
        let tokens: Vec<String> = response.split_whitespace().map(String::from).collect();
        let s = stream::iter(tokens.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
    }

    fn name(&self) -> &'static str {
        "fake-llm"
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-providers --lib testing::fake_llm::tests`

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-providers
git commit -m "feat(providers): FakeLlm with rule table + locale-aware fallback"
```

---

## Task 5: FakeTts

**Files:**
- Create: `crates/athena-voice-providers/src/testing/fake_tts.rs`
- Modify: `crates/athena-voice-providers/src/testing/mod.rs`

**Interfaces:**
- Consumes: `Tts` trait, `Locale`, `SessionId`.
- Produces: `pub struct FakeTts` implementing `Tts`. Behaviour:
    - Emits one `Bytes` chunk per word of `text`, containing the word’s UTF-8 bytes (representing a “packet”).
    - Empty text → empty stream.
    - `name() -> "fake-tts"`.

- [ ] **Step 1: Register module** — append to `testing/mod.rs`:

```rust
pub mod fake_tts;
```

- [ ] **Step 2: Write failing tests in `fake_tts.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::{Locale, SessionId};
    use athena_voice_core::provider::Tts;
    use futures::stream::StreamExt;

    #[tokio::test]
    async fn one_chunk_per_word() {
        let tts = FakeTts::new();
        let mut audio = tts
            .synthesize(SessionId::new_v4(), Locale::new("en").unwrap(), "hello world".into())
            .await
            .unwrap();
        let mut chunks = Vec::new();
        while let Some(c) = audio.next().await {
            chunks.push(c.unwrap());
        }
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref(), b"hello");
        assert_eq!(chunks[1].as_ref(), b"world");
    }

    #[tokio::test]
    async fn empty_text_empty_stream() {
        let tts = FakeTts::new();
        let mut audio = tts
            .synthesize(SessionId::new_v4(), Locale::new("en").unwrap(), String::new())
            .await
            .unwrap();
        assert!(audio.next().await.is_none());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FakeTts::new().name(), "fake-tts");
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-providers --lib testing::fake_tts::tests`

- [ ] **Step 4: Implement `fake_tts.rs`**

```rust
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioStream, BoxError, Tts};

#[derive(Default)]
pub struct FakeTts;

impl FakeTts {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tts for FakeTts {
    async fn synthesize(
        &self,
        _session: SessionId,
        _locale: Locale,
        text: String,
    ) -> Result<AudioStream, BoxError> {
        let chunks: Vec<Bytes> = text
            .split_whitespace()
            .map(|w| Bytes::copy_from_slice(w.as_bytes()))
            .collect();
        let s = stream::iter(chunks.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
    }

    fn name(&self) -> &'static str {
        "fake-tts"
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-providers
git commit -m "feat(providers): FakeTts (one chunk per word)"
```

---

## Task 6: Provider factory + config

**Files:**
- Modify: `crates/athena-voice-providers/src/factory.rs`
- Modify: `crates/athena-voice-providers/src/lib.rs` (re-export `ProviderConfig`)

**Interfaces:**
- Consumes: `Stt`/`Llm`/`Tts` traits (core), `FakeStt/FakeLlm/FakeTts` (Tasks 3–5).
- Produces:
  - `pub struct ProviderConfig { pub stt: StageChoice, pub llm: StageChoice, pub tts: StageChoice }` derived `Deserialize`.
  - `pub enum StageChoice { Fake }` (Plan 2 only exposes `fake`; Plan 3 adds `Ollama`, `MqttRpc`, `Whisper`, `Piper`).
  - `pub struct ProviderFactory { … }` with `new(config: ProviderConfig) -> Self` and methods `stt() -> Arc<dyn Stt>`, `llm() -> Arc<dyn Llm>`, `tts() -> Arc<dyn Tts>`.
  - `StageChoice::Fake` uses default fakes (empty preset, no rules); Task 22 injects real presets in test harness.

- [ ] **Step 1: Write failing test in `factory.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fake_config_roundtrips_toml() {
        let toml_input = r#"
stt = "fake"
llm = "fake"
tts = "fake"
        "#;
        let cfg: ProviderConfig = toml::from_str(toml_input).unwrap();
        assert!(matches!(cfg.stt, StageChoice::Fake));
        assert!(matches!(cfg.llm, StageChoice::Fake));
        assert!(matches!(cfg.tts, StageChoice::Fake));
    }

    #[test]
    fn factory_returns_named_providers() {
        let cfg = ProviderConfig {
            stt: StageChoice::Fake,
            llm: StageChoice::Fake,
            tts: StageChoice::Fake,
        };
        let f = ProviderFactory::new(cfg);
        assert_eq!(f.stt().name(), "fake-stt");
        assert_eq!(f.llm().name(), "fake-llm");
        assert_eq!(f.tts().name(), "fake-tts");
    }
}
```

Note: this test needs `toml` as a dev-dep for the providers crate. Add to `[dev-dependencies]`:
```toml
toml = "0.8"
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-providers --lib factory::tests`

- [ ] **Step 3: Implement `factory.rs`**

```rust
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use athena_voice_core::provider::{Llm, Stt, Tts};

use crate::testing::{fake_llm::FakeLlm, fake_stt::FakeStt, fake_tts::FakeTts};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub stt: StageChoice,
    pub llm: StageChoice,
    pub tts: StageChoice,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageChoice {
    Fake,
    // Ollama, MqttRpc, Whisper, Piper — added in Plan 3
}

pub struct ProviderFactory {
    stt: Arc<dyn Stt>,
    llm: Arc<dyn Llm>,
    tts: Arc<dyn Tts>,
}

impl ProviderFactory {
    #[must_use]
    pub fn new(config: ProviderConfig) -> Self {
        let stt: Arc<dyn Stt> = match config.stt {
            StageChoice::Fake => Arc::new(FakeStt::builder().build()),
        };
        let llm: Arc<dyn Llm> = match config.llm {
            StageChoice::Fake => Arc::new(FakeLlm::builder().build()),
        };
        let tts: Arc<dyn Tts> = match config.tts {
            StageChoice::Fake => Arc::new(FakeTts::new()),
        };
        Self { stt, llm, tts }
    }

    /// Overrides the STT provider (used by integration harnesses to inject a preset FakeStt).
    #[must_use]
    pub fn with_stt(mut self, stt: Arc<dyn Stt>) -> Self {
        self.stt = stt;
        self
    }

    /// Overrides the LLM provider.
    #[must_use]
    pub fn with_llm(mut self, llm: Arc<dyn Llm>) -> Self {
        self.llm = llm;
        self
    }

    /// Overrides the TTS provider.
    #[must_use]
    pub fn with_tts(mut self, tts: Arc<dyn Tts>) -> Self {
        self.tts = tts;
        self
    }

    #[must_use]
    pub fn stt(&self) -> Arc<dyn Stt> { self.stt.clone() }
    #[must_use]
    pub fn llm(&self) -> Arc<dyn Llm> { self.llm.clone() }
    #[must_use]
    pub fn tts(&self) -> Arc<dyn Tts> { self.tts.clone() }
}
```

Update `lib.rs`:
```rust
pub use factory::{ProviderConfig, ProviderFactory, StageChoice};
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-providers
git commit -m "feat(providers): ProviderConfig + ProviderFactory with override hooks"
```

---

## Task 7: MQTT topics + parser

**Files:**
- Create: `crates/athena-voice-runtime/src/mqtt/topics.rs`
- Modify: `crates/athena-voice-runtime/src/mqtt/mod.rs`

**Interfaces:**
- Consumes: `SessionId`, `SatelliteId`.
- Produces:
  - `pub const ROOT: &str = "athena"`.
  - `pub fn sat_wildcard() -> String` → `"athena/sat/+/session/#"`.
  - `pub fn session_transcript(sat: &SatelliteId, sid: SessionId) -> String`.
  - `pub fn session_tts(sat: &SatelliteId, sid: SessionId) -> String`.
  - `pub fn session_tts_meta(sat: &SatelliteId, sid: SessionId) -> String`.
  - `pub fn session_done(sat: &SatelliteId, sid: SessionId) -> String`.
  - `pub fn event_topic(kind: &str) -> String` → `"athena/events/<kind>"`.
  - `pub fn parse_satellite_topic(topic: &str) -> Option<ParsedTopic>` returning one of `Start`, `Audio`, `End`.
  - `pub enum ParsedTopic { Start { sat: SatelliteId, sid: SessionId }, Audio { sat: SatelliteId, sid: SessionId }, End { sat: SatelliteId, sid: SessionId } }`.

- [ ] **Step 1: Update `mqtt/mod.rs`**

```rust
//! MQTT wrapper for Athena-Voice runtime.

pub mod topics;
```

- [ ] **Step 2: Write failing tests in `topics.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::{SatelliteId, SessionId};

    fn sat() -> SatelliteId { SatelliteId::new("phone-01").unwrap() }

    #[test]
    fn wildcard_matches_spec() {
        assert_eq!(sat_wildcard(), "athena/sat/+/session/#");
    }

    #[test]
    fn transcript_topic_layout() {
        let sid = SessionId::new_v4();
        let s = session_transcript(&sat(), sid);
        assert!(s.starts_with("athena/sat/phone-01/session/"));
        assert!(s.ends_with("/transcript"));
        assert!(s.contains(&sid.to_string()));
    }

    #[test]
    fn parse_start() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/start");
        let parsed = parse_satellite_topic(&topic).expect("parses");
        match parsed {
            ParsedTopic::Start { sat, sid: got } => {
                assert_eq!(sat.as_str(), "phone-01");
                assert_eq!(got, sid);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parse_audio() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/audio");
        assert!(matches!(parse_satellite_topic(&topic), Some(ParsedTopic::Audio { .. })));
    }

    #[test]
    fn parse_end() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/end");
        assert!(matches!(parse_satellite_topic(&topic), Some(ParsedTopic::End { .. })));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(parse_satellite_topic("random/topic").is_none());
        assert!(parse_satellite_topic("athena/sat/phone-01/session").is_none());
        assert!(parse_satellite_topic("athena/sat/phone-01/session/not-a-uuid/audio").is_none());
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-runtime --lib mqtt::topics::tests`

- [ ] **Step 4: Implement `topics.rs`**

```rust
use athena_voice_core::ids::{SatelliteId, SessionId};

pub const ROOT: &str = "athena";

#[must_use]
pub fn sat_wildcard() -> String {
    format!("{ROOT}/sat/+/session/#")
}

#[must_use]
pub fn session_transcript(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/transcript")
}

#[must_use]
pub fn session_tts(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/tts")
}

#[must_use]
pub fn session_tts_meta(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/tts/meta")
}

#[must_use]
pub fn session_done(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/done")
}

#[must_use]
pub fn event_topic(kind: &str) -> String {
    format!("{ROOT}/events/{kind}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTopic {
    Start { sat: SatelliteId, sid: SessionId },
    Audio { sat: SatelliteId, sid: SessionId },
    End { sat: SatelliteId, sid: SessionId },
}

#[must_use]
pub fn parse_satellite_topic(topic: &str) -> Option<ParsedTopic> {
    // athena/sat/<sat_id>/session/<sid>/{start|audio|end}
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 6 || parts[0] != ROOT || parts[1] != "sat" || parts[3] != "session" {
        return None;
    }
    let sat = SatelliteId::new(parts[2]).ok()?;
    let sid: SessionId = parts[4].parse().ok()?;
    match parts[5] {
        "start" => Some(ParsedTopic::Start { sat, sid }),
        "audio" => Some(ParsedTopic::Audio { sat, sid }),
        "end"   => Some(ParsedTopic::End   { sat, sid }),
        _ => None,
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): MQTT topic constants + satellite topic parser"
```

---

## Task 8: MQTT client wrapper

**Files:**
- Modify: `crates/athena-voice-runtime/src/mqtt/mod.rs`
- Create: `crates/athena-voice-runtime/tests/mqtt_roundtrip.rs`

**Interfaces:**
- Consumes: `rumqttc`, `rumqttd` (dev), topic helpers (Task 7).
- Produces:
  - `pub struct MqttClient { pub tx: rumqttc::AsyncClient, event_loop: Arc<Mutex<rumqttc::EventLoop>> }` — wrapper around `AsyncClient` + event loop.
  - `pub struct MqttConfig { pub host: String, pub port: u16, pub client_id: String, pub username: Option<String>, pub password: Option<String>, pub keep_alive_secs: u64 }`.
  - `pub async fn connect(config: MqttConfig) -> Result<MqttClient, RuntimeError>`.
  - `pub fn spawn_event_pump(client: MqttClient, on_publish: impl FnMut(rumqttc::Publish)) -> JoinHandle<()>` — drives the event loop, dispatches Publish events to the callback.

- [ ] **Step 1: Write the RuntimeError skeleton** — `crates/athena-voice-runtime/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("mqtt: {0}")]
    Mqtt(#[from] rumqttc::ConnectionError),
    #[error("mqtt client: {0}")]
    MqttClient(#[from] rumqttc::ClientError),
    #[error("locale pack: {0}")]
    Locale(String),
    #[error("config: {0}")]
    Config(String),
    #[error("shutdown")]
    Shutdown,
}
```

- [ ] **Step 2: Write integration test in `tests/mqtt_roundtrip.rs`**

```rust
use std::time::Duration;

use athena_voice_runtime::mqtt::{MqttClient, MqttConfig};
use rumqttc::{QoS, Publish};
use tokio::sync::mpsc;

// Spawns rumqttd on a random port, returns the port.
async fn spawn_broker() -> u16 {
    // Minimal embedded broker for tests.
    let port = portpicker::pick_unused_port().expect("free port");
    let config = rumqttd::Config {
        v4: Some(std::collections::HashMap::from([
            ("v4-1".to_string(), rumqttd::ServerSettings {
                name: "v4-1".into(),
                listen: format!("0.0.0.0:{port}").parse().unwrap(),
                tls: None,
                next_connection_delay_ms: 1,
                connections: rumqttd::ConnectionSettings {
                    connection_timeout_ms: 5000,
                    max_payload_size: 1_048_576,
                    max_inflight_count: 100,
                    auth: None,
                    dynamic_filters: false,
                    external_auth: None,
                },
            })
        ])),
        ..Default::default()
    };
    let mut broker = rumqttd::Broker::new(config);
    tokio::spawn(async move {
        let _ = broker.start();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    port
}

#[tokio::test]
async fn publish_and_receive_roundtrip() {
    let port = spawn_broker().await;
    let (tx, mut rx) = mpsc::unbounded_channel::<Publish>();

    let mut sub_client = MqttClient::connect(MqttConfig {
        host: "127.0.0.1".into(),
        port,
        client_id: "sub".into(),
        username: None,
        password: None,
        keep_alive_secs: 30,
    })
    .await
    .expect("connect sub");
    sub_client.tx.subscribe("test/topic", QoS::AtLeastOnce).await.unwrap();

    tokio::spawn(async move {
        loop {
            let ev = sub_client
                .event_loop
                .lock()
                .await
                .poll()
                .await;
            if let Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(p))) = ev {
                let _ = tx.send(p);
            }
        }
    });

    let pub_client = MqttClient::connect(MqttConfig {
        host: "127.0.0.1".into(),
        port,
        client_id: "pub".into(),
        username: None,
        password: None,
        keep_alive_secs: 30,
    })
    .await
    .expect("connect pub");
    pub_client.tx.publish("test/topic", QoS::AtLeastOnce, false, b"hello".as_slice()).await.unwrap();
    // pump the pub client's event loop
    tokio::spawn(async move {
        loop {
            let _ = pub_client.event_loop.lock().await.poll().await;
        }
    });

    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.unwrap().unwrap();
    assert_eq!(received.topic, "test/topic");
    assert_eq!(&received.payload[..], b"hello");
}
```

Add dev-deps in `crates/athena-voice-runtime/Cargo.toml`:
```toml
portpicker = "0.1"
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-runtime --test mqtt_roundtrip`
Expected: `MqttClient` unresolved.

- [ ] **Step 4: Implement `mqtt/mod.rs`**

```rust
pub mod topics;

use std::sync::Arc;

use rumqttc::{AsyncClient, EventLoop, MqttOptions};
use tokio::sync::Mutex;

use crate::error::RuntimeError;

#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub keep_alive_secs: u64,
}

pub struct MqttClient {
    pub tx: AsyncClient,
    pub event_loop: Arc<Mutex<EventLoop>>,
}

impl MqttClient {
    pub async fn connect(config: MqttConfig) -> Result<Self, RuntimeError> {
        let mut opts = MqttOptions::new(&config.client_id, &config.host, config.port);
        opts.set_keep_alive(std::time::Duration::from_secs(config.keep_alive_secs));
        if let (Some(u), Some(p)) = (&config.username, &config.password) {
            opts.set_credentials(u, p);
        }
        let (tx, event_loop) = AsyncClient::new(opts, 128);
        Ok(Self { tx, event_loop: Arc::new(Mutex::new(event_loop)) })
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-runtime --test mqtt_roundtrip`
Expected: 1 test passes (may take ~2 s for the broker to boot).

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): MqttClient wrapper + connect + embedded-broker roundtrip test"
```

---

## Task 9: SessionManager

**Files:**
- Modify: `crates/athena-voice-runtime/src/session/mod.rs`
- Create: `crates/athena-voice-runtime/src/session/manager.rs`

**Interfaces:**
- Consumes: `SessionId`, `SatelliteId`, `Locale`.
- Produces:
  - `pub struct SessionState { pub sat: SatelliteId, pub locale: Locale, pub cancel: CancellationToken, pub audio_tx: mpsc::Sender<AudioFrame> }`.
  - `pub struct SessionManager { map: DashMap<SessionId, SessionState> }`.
  - Methods:
    - `open(sid, sat, locale, audio_tx) -> Result<(), SessionExists>` — inserts a new state; returns error on duplicate.
    - `get(sid) -> Option<Ref<SessionId, SessionState>>`.
    - `close(sid)` — removes and cancels.
    - `cancel_all()` — cancels every state's token (used at shutdown).
    - `len() -> usize`.

- [ ] **Step 1: Update `session/mod.rs`**

```rust
pub mod manager;
pub use manager::{SessionExists, SessionManager, SessionState};
```

- [ ] **Step 2: Write failing tests in `manager.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
    use tokio::sync::mpsc;

    fn state() -> (SessionId, SatelliteId, Locale, mpsc::Sender<athena_voice_core::types::AudioFrame>) {
        let (tx, _rx) = mpsc::channel(1);
        (
            SessionId::new_v4(),
            SatelliteId::new("phone-01").unwrap(),
            Locale::new("fr").unwrap(),
            tx,
        )
    }

    #[tokio::test]
    async fn open_and_get() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx) = state();
        mgr.open(sid, sat.clone(), loc.clone(), tx).unwrap();
        assert_eq!(mgr.len(), 1);
        let entry = mgr.get(sid).expect("present");
        assert_eq!(entry.sat, sat);
        assert_eq!(entry.locale, loc);
    }

    #[tokio::test]
    async fn open_duplicate_returns_error() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx) = state();
        mgr.open(sid, sat.clone(), loc.clone(), tx.clone()).unwrap();
        assert!(matches!(
            mgr.open(sid, sat, loc, tx),
            Err(SessionExists { .. })
        ));
    }

    #[tokio::test]
    async fn close_cancels_and_removes() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx) = state();
        mgr.open(sid, sat, loc, tx).unwrap();
        let token = mgr.get(sid).unwrap().cancel.clone();
        mgr.close(sid);
        assert_eq!(mgr.len(), 0);
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancel_all_fires_every_token() {
        let mgr = SessionManager::default();
        let mut tokens = Vec::new();
        for _ in 0..3 {
            let (sid, sat, loc, tx) = state();
            mgr.open(sid, sat, loc, tx).unwrap();
            tokens.push(mgr.get(sid).unwrap().cancel.clone());
        }
        mgr.cancel_all();
        for t in tokens {
            assert!(t.is_cancelled());
        }
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-runtime --lib session::manager::tests`

- [ ] **Step 4: Implement `manager.rs`**

```rust
use dashmap::DashMap;
use dashmap::mapref::one::Ref;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_core::types::AudioFrame;

#[derive(Debug, Error)]
#[error("session {session} already exists")]
pub struct SessionExists {
    pub session: SessionId,
}

pub struct SessionState {
    pub sat: SatelliteId,
    pub locale: Locale,
    pub cancel: CancellationToken,
    pub audio_tx: mpsc::Sender<AudioFrame>,
}

#[derive(Default)]
pub struct SessionManager {
    map: DashMap<SessionId, SessionState>,
}

impl SessionManager {
    pub fn open(
        &self,
        session: SessionId,
        sat: SatelliteId,
        locale: Locale,
        audio_tx: mpsc::Sender<AudioFrame>,
    ) -> Result<(), SessionExists> {
        if self.map.contains_key(&session) {
            return Err(SessionExists { session });
        }
        self.map.insert(
            session,
            SessionState { sat, locale, cancel: CancellationToken::new(), audio_tx },
        );
        Ok(())
    }

    #[must_use]
    pub fn get(&self, session: SessionId) -> Option<Ref<'_, SessionId, SessionState>> {
        self.map.get(&session)
    }

    pub fn close(&self, session: SessionId) {
        if let Some((_, state)) = self.map.remove(&session) {
            state.cancel.cancel();
        }
    }

    pub fn cancel_all(&self) {
        for entry in self.map.iter() {
            entry.value().cancel.cancel();
        }
    }

    #[must_use]
    pub fn len(&self) -> usize { self.map.len() }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
}
```

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): SessionManager with per-session cancellation tokens"
```

---

## Task 10: Event bus

**Files:**
- Modify: `crates/athena-voice-runtime/src/event_bus.rs`
- Create: `crates/athena-voice-runtime/tests/event_bus.rs`

**Interfaces:**
- Consumes: `Event` (core), `MqttClient` (Task 8).
- Produces:
  - `pub struct EventBus { tx: broadcast::Sender<Event> }`.
  - `pub fn new(capacity: usize) -> Self` (default cap 1024).
  - `pub fn sender(&self) -> broadcast::Sender<Event>`.
  - `pub fn subscribe(&self) -> broadcast::Receiver<Event>`.
  - `pub fn spawn_mqtt_mirror(bus: broadcast::Sender<Event>, mqtt: AsyncClient) -> JoinHandle<()>` — spawns a task that subscribes to the bus, serialises each event to JSON, publishes to `athena/events/<kind>` with QoS 1.

- [ ] **Step 1: Write failing test in `tests/event_bus.rs`**

```rust
use athena_voice_core::event::Event;
use athena_voice_core::ids::SessionId;
use athena_voice_runtime::event_bus::EventBus;

#[tokio::test]
async fn subscribers_receive_broadcast() {
    let bus = EventBus::new(16);
    let mut rx = bus.subscribe();
    let ev = Event::LlmFallback { session: SessionId::new_v4() };
    bus.sender().send(ev).unwrap();
    let got = rx.recv().await.unwrap();
    assert!(matches!(got, Event::LlmFallback { .. }));
}

#[tokio::test]
async fn lagged_receiver_gets_lagged_error_then_recovers() {
    let bus = EventBus::new(2);
    let mut rx = bus.subscribe();
    for _ in 0..5 {
        bus.sender()
            .send(Event::LlmFallback { session: SessionId::new_v4() })
            .unwrap();
    }
    // First recv should indicate lag (RecvError::Lagged)
    let first = rx.recv().await;
    assert!(matches!(first, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))));
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-runtime --test event_bus`

- [ ] **Step 3: Implement `event_bus.rs`**

```rust
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

use athena_voice_core::event::Event;

pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    #[must_use]
    pub fn sender(&self) -> broadcast::Sender<Event> {
        self.tx.clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

/// Spawns a task that consumes broadcast events and publishes each as JSON on
/// `athena/events/<kind>`. Returns the JoinHandle so the caller can shut it down.
pub fn spawn_mqtt_mirror(
    tx: broadcast::Sender<Event>,
    mqtt: rumqttc::AsyncClient,
) -> JoinHandle<()> {
    let mut rx = tx.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let value = match serde_json::to_value(&event) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "failed to serialise Event for MQTT mirror");
                            continue;
                        }
                    };
                    let kind = value
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let payload = match serde_json::to_vec(&value) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(error = %e, "failed to serialise Event to bytes");
                            continue;
                        }
                    };
                    let topic = crate::mqtt::topics::event_topic(&kind);
                    if let Err(e) =
                        mqtt.publish(topic, rumqttc::QoS::AtLeastOnce, false, payload).await
                    {
                        warn!(error = %e, "mqtt mirror publish failed");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "event_bus mirror lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
```

- [ ] **Step 4: Run tests, verify pass**

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): EventBus (broadcast) + MQTT mirror task"
```

---

## Task 11: Pipeline actors — Ingest + Vad

**Files:**
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs`
- Create: `crates/athena-voice-runtime/src/pipeline/ingest.rs`
- Create: `crates/athena-voice-runtime/src/pipeline/vad.rs`

**Interfaces:**
- Consumes: `AudioFrame` (core), `CancellationToken`.
- Produces:
  - `pub fn spawn_ingest(rx: mpsc::Receiver<AudioFrame>, tx: mpsc::Sender<AudioFrame>, cancel: CancellationToken) -> JoinHandle<()>`.
    - Ingest passes frames through; sanity-checks that `frame.pcm.len()` is non-zero. On zero-length frame, drops silently.
  - `pub fn spawn_vad(rx: mpsc::Receiver<AudioFrame>, tx: mpsc::Sender<AudioFrame>, cancel: CancellationToken, endpoint_after_silent_frames: u32) -> JoinHandle<()>`.
    - Plan 2 VAD is trivial: passthrough. Silent-frame counting scaffolding present but the real detector lands in Plan 3.

- [ ] **Step 1: `pipeline/mod.rs`**

```rust
pub mod ingest;
pub mod vad;
```

- [ ] **Step 2: Write failing tests in `ingest.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::ids::SessionId;
    use athena_voice_core::types::AudioFrame;
    use bytes::Bytes;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn frame(session: SessionId, seq: u32, pcm: &[u8]) -> AudioFrame {
        AudioFrame { session, seq, pcm: Bytes::copy_from_slice(pcm) }
    }

    #[tokio::test]
    async fn passes_through_non_empty_frames() {
        let (in_tx, in_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        let handle = spawn_ingest(in_rx, out_tx, CancellationToken::new());

        let sid = SessionId::new_v4();
        in_tx.send(frame(sid, 0, &[1, 2, 3])).await.unwrap();
        drop(in_tx);
        let received = out_rx.recv().await.unwrap();
        assert_eq!(received.seq, 0);
        assert!(out_rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drops_empty_frames() {
        let (in_tx, in_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        spawn_ingest(in_rx, out_tx, CancellationToken::new());

        let sid = SessionId::new_v4();
        in_tx.send(frame(sid, 0, &[])).await.unwrap();
        in_tx.send(frame(sid, 1, &[1, 2])).await.unwrap();
        drop(in_tx);
        let first = out_rx.recv().await.unwrap();
        assert_eq!(first.seq, 1);
        assert!(out_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn cancellation_terminates() {
        let (_in_tx, in_rx) = mpsc::channel::<AudioFrame>(4);
        let (out_tx, _out_rx) = mpsc::channel(4);
        let token = CancellationToken::new();
        let handle = spawn_ingest(in_rx, out_tx, token.clone());
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("timed out")
            .unwrap();
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-runtime --lib pipeline::ingest::tests`

- [ ] **Step 4: Implement `ingest.rs`**

```rust
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::types::AudioFrame;

pub fn spawn_ingest(
    mut rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<AudioFrame>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(frame) => {
                        if frame.pcm.is_empty() {
                            continue;
                        }
                        if tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    })
}
```

- [ ] **Step 5: Implement `vad.rs` (passthrough placeholder)**

```rust
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::types::AudioFrame;

/// Plan 2 VAD is passthrough. Plan 3 upgrades to an energy or Silero-VAD based
/// endpoint detector. The `_endpoint_after_silent_frames` parameter documents
/// where the future threshold will live.
pub fn spawn_vad(
    mut rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<AudioFrame>,
    cancel: CancellationToken,
    _endpoint_after_silent_frames: u32,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(frame) => {
                        if tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    })
}
```

Add a minimal test mirroring the passthrough shape (analogous to ingest's `passes_through_non_empty_frames` + `cancellation_terminates`).

- [ ] **Step 6: Run all tests, verify pass**

Run: `cargo test -p athena-voice-runtime --lib pipeline`

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): Ingest + Vad passthrough actors with cancellation"
```

---

## Task 12: Pipeline actor — Stt

**Files:**
- Create: `crates/athena-voice-runtime/src/pipeline/stt.rs`
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs`

**Interfaces:**
- Consumes: `Stt` trait (core), `AudioFrame`, `Transcript`, `Locale`, `SessionId`, `CancellationToken`.
- Produces:
  - `pub fn spawn_stt(session: SessionId, locale: Locale, stt: Arc<dyn Stt>, rx: mpsc::Receiver<AudioFrame>, tx: mpsc::Sender<Transcript>, event_tx: broadcast::Sender<Event>, cancel: CancellationToken) -> JoinHandle<()>`.
    - Converts the `AudioFrame` receiver into a `Stream<Item = AudioFrame>`, calls `Stt::transcribe`, forwards each emitted `Transcript` to `tx` and emits `Event::TranscriptPartial` / `Event::TranscriptFinal` on the bus.

- [ ] **Step 1: Add `pub mod stt;` in `pipeline/mod.rs`**

- [ ] **Step 2: Write failing tests in `stt.rs`**

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use athena_voice_core::event::Event;
    use athena_voice_core::ids::{Locale, SessionId};
    use athena_voice_core::types::{AudioFrame, Transcript};
    use athena_voice_providers::testing::fake_stt::FakeStt;
    use bytes::Bytes;
    use tokio::sync::{broadcast, mpsc};
    use tokio_util::sync::CancellationToken;

    fn preset_stt(sid: SessionId) -> Arc<dyn athena_voice_core::provider::Stt> {
        Arc::new(
            FakeStt::builder()
                .preset(
                    sid,
                    vec![
                        Transcript { text: "bon".into(), is_final: false, confidence: None },
                        Transcript { text: "bonjour".into(), is_final: true, confidence: None },
                    ],
                )
                .build(),
        )
    }

    #[tokio::test]
    async fn emits_transcripts_and_events() {
        let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(4);
        let (t_tx, mut t_rx) = mpsc::channel::<Transcript>(4);
        let (ev_tx, mut ev_rx) = broadcast::channel::<Event>(16);
        let sid = SessionId::new_v4();
        let stt = preset_stt(sid);

        let handle = spawn_stt(
            sid,
            Locale::new("fr").unwrap(),
            stt,
            audio_rx,
            t_tx,
            ev_tx,
            CancellationToken::new(),
        );

        // Send one frame to ensure the actor consumes; drop tx to close audio.
        audio_tx.send(AudioFrame { session: sid, seq: 0, pcm: Bytes::from_static(&[1]) })
            .await
            .unwrap();
        drop(audio_tx);

        let first_transcript = t_rx.recv().await.unwrap();
        assert_eq!(first_transcript.text, "bon");
        assert!(!first_transcript.is_final);
        let second_transcript = t_rx.recv().await.unwrap();
        assert!(second_transcript.is_final);
        assert!(t_rx.recv().await.is_none());
        handle.await.unwrap();

        let mut kinds = Vec::new();
        while let Ok(ev) = ev_rx.try_recv() {
            kinds.push(match ev {
                Event::TranscriptPartial { .. } => "partial",
                Event::TranscriptFinal { .. } => "final",
                _ => "other",
            });
        }
        assert!(kinds.contains(&"partial"));
        assert!(kinds.contains(&"final"));
    }
}
```

- [ ] **Step 3: Verify fail**

- [ ] **Step 4: Implement `stt.rs`**

```rust
use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Stt;
use athena_voice_core::types::{AudioFrame, Transcript};

pub fn spawn_stt(
    session: SessionId,
    locale: Locale,
    stt: Arc<dyn Stt>,
    rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<Transcript>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let audio_stream = ReceiverStream::new(rx);
        let boxed: athena_voice_core::provider::AudioFrameStream = Box::pin(audio_stream);
        let mut ts = match stt.transcribe(session, locale.clone(), boxed).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, "stt provider transcribe returned error");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                maybe = ts.next() => match maybe {
                    Some(Ok(t)) => {
                        let ev = if t.is_final {
                            Event::TranscriptFinal { session, text: t.text.clone() }
                        } else {
                            Event::TranscriptPartial { session, text: t.text.clone() }
                        };
                        let _ = event_tx.send(ev);
                        if tx.send(t).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        warn!(error = %err, "stt stream error");
                        break;
                    }
                    None => break,
                }
            }
        }
    })
}
```

Add dep in `Cargo.toml`:
```toml
tokio-stream = "0.1"
```

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): Stt actor drains provider stream, emits events"
```

---

## Task 13: Pipeline actor — IntentRouter + Llm

**Files:**
- Create: `crates/athena-voice-runtime/src/pipeline/router.rs`
- Create: `crates/athena-voice-runtime/src/pipeline/llm.rs`
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs`

**Interfaces:**
- Router: `pub fn spawn_router(rx: mpsc::Receiver<Transcript>, llm_tx: mpsc::Sender<String>, event_tx: broadcast::Sender<Event>, session: SessionId, cancel: CancellationToken)`.
    - Plan 2: only forwards *final* transcripts to `llm_tx` (partials are dropped downstream). Emits `Event::LlmFallback` on every final.
- Llm actor: `pub fn spawn_llm(session: SessionId, locale: Locale, llm: Arc<dyn Llm>, prompt_rx: mpsc::Receiver<String>, token_tx: mpsc::Sender<String>, cancel: CancellationToken)`.
    - Consumes final-transcript prompts; calls `Llm::complete`, forwards tokens to `token_tx`.

- [ ] **Step 1: Register modules**

`pipeline/mod.rs`:
```rust
pub mod llm;
pub mod router;
```

- [ ] **Step 2: Write failing tests in `router.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_core::event::Event;
    use athena_voice_core::ids::SessionId;
    use athena_voice_core::types::Transcript;
    use tokio::sync::{broadcast, mpsc};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn only_finals_reach_llm_and_emit_fallback() {
        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, mut llm_rx) = mpsc::channel(4);
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let sid = SessionId::new_v4();
        let handle = spawn_router(t_rx, llm_tx, ev_tx, sid, CancellationToken::new());

        t_tx.send(Transcript { text: "bon".into(), is_final: false, confidence: None })
            .await
            .unwrap();
        t_tx.send(Transcript { text: "bonjour".into(), is_final: true, confidence: None })
            .await
            .unwrap();
        drop(t_tx);

        let prompt = llm_rx.recv().await.unwrap();
        assert_eq!(prompt, "bonjour");
        assert!(llm_rx.recv().await.is_none());
        handle.await.unwrap();

        let mut got_fallback = false;
        while let Ok(ev) = ev_rx.try_recv() {
            if matches!(ev, Event::LlmFallback { .. }) {
                got_fallback = true;
            }
        }
        assert!(got_fallback);
    }
}
```

Similarly in `llm.rs` — test that FakeLlm(rule) sees prompt "quel est le weather" and emits `["il", "fait", "beau"]` tokens.

- [ ] **Step 3: Verify fail**

- [ ] **Step 4: Implement `router.rs`**

```rust
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::event::Event;
use athena_voice_core::ids::SessionId;
use athena_voice_core::types::Transcript;

pub fn spawn_router(
    mut rx: mpsc::Receiver<Transcript>,
    llm_tx: mpsc::Sender<String>,
    event_tx: broadcast::Sender<Event>,
    session: SessionId,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(t) if t.is_final => {
                        let _ = event_tx.send(Event::LlmFallback { session });
                        if llm_tx.send(t.text).await.is_err() {
                            break;
                        }
                    }
                    Some(_) => {} // partials dropped in Plan 2
                    None => break,
                }
            }
        }
    })
}
```

Implement `llm.rs` symmetrically using `Llm::complete`.

- [ ] **Step 5: Run tests, verify pass**

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): IntentRouter (LLM-fallback only) + Llm actor"
```

---

## Task 14: Pipeline actor — Tts + ResponseSink

**Files:**
- Create: `crates/athena-voice-runtime/src/pipeline/tts.rs`
- Create: `crates/athena-voice-runtime/src/pipeline/sink.rs`
- Modify: `crates/athena-voice-runtime/src/pipeline/mod.rs`

**Interfaces:**
- Tts actor: `pub fn spawn_tts(session, locale, tts: Arc<dyn Tts>, token_rx: mpsc::Receiver<String>, chunk_tx: mpsc::Sender<Bytes>, event_tx: broadcast::Sender<Event>, cancel)`.
    - Buffers tokens by sentence boundary (`.`, `?`, `!`, or 100 chars). For each sentence, calls `Tts::synthesize`, forwards chunks to `chunk_tx`, emits `Event::TtsChunk`.
- ResponseSink: `pub fn spawn_sink(session, sat: SatelliteId, mqtt: AsyncClient, chunk_rx: mpsc::Receiver<Bytes>, event_tx: broadcast::Sender<Event>, cancel)`.
    - First chunk: publishes `session/<sid>/tts/meta` JSON. Then publishes each chunk as raw bytes to `session/<sid>/tts`. On channel close, publishes `session/<sid>/done` and emits `Event::SessionEnded { outcome: Ok }`.

- [ ] **Step 1: Register modules**

- [ ] **Step 2: Failing tests + impls per usual TDD** — because they are long, use the same TDD template as Task 13. Cover:
    - `tts_buffers_by_sentence_and_synthesises_each`
    - `sink_publishes_meta_before_first_tts_chunk` (integration test spawning broker; assert MQTT publish order)
    - `sink_emits_session_ended_on_close`

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-runtime
git commit -m "feat(runtime): Tts + ResponseSink actors (meta + chunks + done)"
```

---

## Task 15: Satellite adapter — ingress

**Files:**
- Modify: `crates/athena-voice-runtime/src/satellite/mod.rs`
- Create: `crates/athena-voice-runtime/src/satellite/ingress.rs`

**Interfaces:**
- Consumes: `MqttClient`, `SessionManager`, `EventBus`, `ProviderFactory`, `Store`.
- Produces: `pub fn spawn_ingress(deps: SatelliteDeps) -> JoinHandle<()>`.
    - Subscribes to `athena/sat/+/session/#`, polls the event loop.
    - On `Start`: parse JSON, `SessionManager::open`, spawn the full pipeline actor chain (Tasks 11–14) for this session.
    - On `Audio`: look up session, push `AudioFrame` to the session's `audio_tx`.
    - On `End`: close the session (audio channel drops → pipeline flushes).

- [ ] Detailed implementation and TDD steps follow the same shape as previous tasks. Deliverable: an integration test that publishes `session/start` + one audio chunk + `session/end` and observes the `SessionEnded` event on the bus, all with fake providers.

- [ ] Commit.

---

## Task 16: Satellite adapter — egress

**Files:**
- Create: `crates/athena-voice-runtime/src/satellite/egress.rs`

**Interfaces:**
- Consumes: `EventBus` receiver, `MqttClient`.
- Produces: `pub fn spawn_egress(deps: SatelliteDeps) -> JoinHandle<()>` — subscribes to the event bus and publishes to satellite egress topics on `TranscriptPartial`/`TranscriptFinal` (→ `session/<sid>/transcript`) and `SessionEnded` (→ `session/<sid>/done`).

- [ ] TDD steps analogous. Commit.

---

## Task 17: Extended CLI config

**Files:**
- Modify: `crates/athena-voice-cli/src/config.rs`
- Modify: `athena.example.toml`
- Modify: `crates/athena-voice-cli/tests/config.rs`

**Interfaces:**
- Adds to `Config`:
  - `pub mqtt: MqttConfig` (host, port, client_id, username, password, keep_alive_secs)
  - `pub providers: ProviderConfig`
- Extends example config with `[mqtt]` and `[providers]` sections.

- [ ] Tests updated to check new fields parse. Commit.

---

## Task 18: `serve::run` body — build the actor DAG

**Files:**
- Modify: `crates/athena-voice-cli/src/serve.rs`
- Add: `crates/athena-voice-runtime/src/config.rs` (`RuntimeConfig`) and top-level `Runtime::spawn` in `lib.rs`.

**Interfaces:**
- `pub struct Runtime { manager, event_bus, mqtt, join_handles }`.
- `pub async fn spawn(cfg: RuntimeConfig, factory: ProviderFactory, store: Arc<dyn Store>) -> Result<Runtime, RuntimeError>` — connects MQTT, spawns satellite ingress + egress + event mirror.
- `pub async fn shutdown(self)` — cancels all sessions, drops handles.
- `serve::run` (non-dry-run branch): build config → `ProviderFactory::new(cfg.providers)` → `SqliteStore::open` → `Runtime::spawn` → `tokio::signal::ctrl_c().await` → `runtime.shutdown().await`.

- [ ] Integration test in `crates/athena-voice-runtime/tests/end_to_end.rs`:

```rust
// pseudocode:
// spawn broker on random port
// build RuntimeConfig using that port + fake providers
// spawn Runtime
// simulate a satellite: publish session/start + one audio + session/end
// subscribe to athena/sat/phone-01/session/<sid>/done, wait for it
// assert response_text non-empty
```

- [ ] Commit.

---

## Task 19: Locale packs (bootstrap)

**Files:**
- Create: `locales/fr.toml`
- Create: `locales/en.toml`
- Modify: `crates/athena-voice-runtime/src/locale.rs`

**Interfaces:**
- `pub struct LocalePack { pub locale: Locale, pub llm_system_prompt: String, pub error_phrases: HashMap<String, String> }`.
- `pub fn load_pack(path: &Path) -> Result<LocalePack, RuntimeError>`.
- Pack format (TOML):
  ```toml
  locale = "fr"
  llm_system_prompt = "Tu es un assistant vocal utile. …"

  [error_phrases]
  stt_unavailable = "Je n'ai pas pu vous entendre."
  llm_unavailable = "Mon cerveau est hors ligne."
  tts_unavailable = "Désolé, je ne peux pas parler."
  overloaded      = "Le système est occupé, réessayez."
  ```
- Tests: parse both packs; assert required keys present; validate `Locale::new(locale)` succeeds.

- [ ] Commit.

---

## Task 20: End-to-end golden test

**Files:**
- Create: `crates/athena-voice-runtime/tests/golden_llm_fallback.rs`

**Interfaces:**
- Spawns broker + runtime with `FakeStt` presetting a French transcript and `FakeLlm` with a matching rule.
- Drives a fake satellite through the full flow.
- Asserts:
  - Received `session/<sid>/transcript` with `is_final:true`.
  - Received `session/<sid>/tts/meta` first, then ≥1 `session/<sid>/tts` chunk.
  - Received `session/<sid>/done { outcome: "ok", response_text: <expected> }`.
  - Event log: `[SessionStarted, TranscriptPartial?, TranscriptFinal, LlmFallback, TtsChunk×N, SessionEnded]`.

- [ ] Commit.

---

## Task 21: Cancellation + concurrency tests

**Files:**
- Create: `crates/athena-voice-runtime/tests/cancellation.rs`
- Create: `crates/athena-voice-runtime/tests/concurrent_sessions.rs`

**Interfaces:**
- Cancellation: fake satellite publishes `session/end { reason: "cancel" }` mid-flow; assert `session/done { outcome: "cancelled" }` arrives within 500 ms; assert no further TTS chunks.
- Concurrency: 3 satellites simultaneously; assert 3 distinct `session/done` outcomes; no cross-talk in transcript topics.

- [ ] Commit.

---

## Task 22: Fmt + clippy sweep, CI pass

**Files:**
- Whatever needs to move.

- [ ] Run `cargo fmt --all` and `cargo clippy --workspace --all-features --all-targets -- -D warnings`; commit whatever's left.
- [ ] Push and confirm the CI workflow (from Plan 1 Task 20) is still green with the new crates + tests.

---

## Definition of Done for Plan 2

Plan 2 is complete when **all** are true:

1. `cargo build --workspace` clean.
2. `cargo test --workspace` — all tests green. Target: ~85 tests across the workspace (Plan 1's 48 + ~35 new).
3. `cargo clippy --workspace --all-features --all-targets -- -D warnings` — clean.
4. `cargo fmt --all --check` — clean.
5. Golden end-to-end test (`tests/golden_llm_fallback.rs`) passes: fake satellite drives a full session, event sequence and MQTT publishes are as expected.
6. Cancellation and concurrency tests pass.
7. `athena-voice serve --config <path>` (no `--dry-run`) boots a real runtime against a co-located mosquitto broker (verified manually with `docker run -p 1883:1883 eclipse-mosquitto:2 mosquitto -v` and MQTT Explorer or an equivalent client). *This manual step is per-release acceptance, not per-PR CI.*
8. GitHub Actions CI green on the branch.
9. Docs updated: `docs/superpowers/plans/2026-07-10-athena-voice-pipeline-fakes.md` (this file) exists and is committed.

## Explicitly deferred to later plans

- Real STT / LLM / TTS providers (Plan 3).
- WASM host + skill loading + pattern rule matcher (Plan 4).
- Dashboard (Plan 5).
- Docker Compose ops (Plan 5).
- Multi-arch CI (Plan 5).

These are intentional non-goals. Do not sneak them into Plan 2 tasks.
