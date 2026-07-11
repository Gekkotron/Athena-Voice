# Athena-Voice — Plan 3: Remote Providers

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `FakeLlm` in the default runtime with an **Ollama HTTP client**, and add generic **MQTT-provider clients** for STT and TTS that any external language-agnostic service can implement (subscribe to `athena/providers/{stt,tts}/<name>/request`, publish results). Also add a **retry decorator** and **circuit breaker** so the runtime survives transient provider failures. After this plan, `athena-voice serve` (with `providers.llm = "ollama"`) will drive real conversational answers against a locally-running Ollama container.

**Architecture:** The `Fake*` providers stay (kept as default, and used in unit tests). Two new module trees under `athena-voice-providers/src/remote/`:

- `ollama.rs` — `OllamaLlm` implementing `Llm`. Uses `reqwest` to call `/api/chat` on the Ollama HTTP endpoint. Streams tokens back via server-sent-events (`stream: true`).
- `mqtt_client.rs` — request/reply glue over MQTT for a stage: publish JSON to `athena/providers/<stage>/<name>/request`, subscribe to `athena/providers/<stage>/<name>/response`, correlate by session_id.
- `mqtt_stt.rs`, `mqtt_tts.rs` — thin wrappers that expose the MQTT-provider client through the `Stt` / `Tts` traits.

Two orthogonal cross-cutting pieces:

- `circuit.rs` — `CircuitBreaker` with `Closed → Open → HalfOpen` state machine per (stage, provider name). 5 failures within 30 s → Open for 60 s; probe every 15 s in HalfOpen; success closes.
- `retry.rs` — decorator that wraps `Arc<dyn Stt/Llm/Tts>` with N retries + backoff. Consults circuit before each call. Records failures.

**Tech Stack:** `reqwest` 0.12 (Ollama HTTP), `mockito` 1.5 (dev-dep — mock Ollama for tests). Same versions already in the workspace.

## Global Constraints

- **Only remote providers.** Local `whisper-rs` / `llama-cpp-rs` / `piper-rs` bindings ship in Plan 3.1 (deferred; the spec allows it).
- **Ollama is the only concrete LLM provider in Plan 3.** OpenAI-compatible is a small variant — deferred to Plan 3.1 if requested.
- **STT + TTS get MQTT-provider clients only** (no HTTP variants for these). Rationale: STT and TTS are typically self-hosted services and MQTT is our chosen internal bus.
- **`Fake*` providers remain unchanged.** They must still pass every existing test.
- **No topology changes to the actor DAG.** Only the `Stt`/`Llm`/`Tts` trait-object handed to the actors changes.
- **`StageChoice` enum grows** but stays backward-compatible: new variants `Ollama`, `MqttStt`, `MqttTts`. Old `Fake` still works.
- **Retry + circuit apply to *any* provider,** including fakes (they're a decorator around the trait).
- **New workspace dependencies** live in `[workspace.dependencies]` and are consumed via `{ workspace = true }`.
- Same edition/toolchain/attribution rules as Plans 1 and 2.

## File structure produced by this plan

```
athena-voice/
├── Cargo.toml                                            # (mod) add reqwest + mockito
├── athena.example.toml                                    # (mod) document new StageChoice variants
├── crates/
│   └── athena-voice-providers/
│       ├── Cargo.toml                                    # (mod) reqwest, mockito
│       └── src/
│           ├── circuit.rs                                # (new) circuit breaker
│           ├── retry.rs                                  # (new) retry decorator
│           ├── factory.rs                                # (mod) extended StageChoice + factory arms
│           └── remote/
│               ├── mod.rs                                # (new)
│               ├── ollama.rs                             # (new) OllamaLlm impls Llm
│               ├── mqtt_client.rs                        # (new) request/reply MQTT client
│               ├── mqtt_stt.rs                           # (new) MqttStt impls Stt
│               └── mqtt_tts.rs                           # (new) MqttTts impls Tts
```

---

## Task 1: Add `reqwest` + `mockito` deps, provider `remote/` module skeleton

- [ ] **Step 1: Root `Cargo.toml`**

Add to `[workspace.dependencies]`:
```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
```

Add to `[workspace.dev-dependencies]` — wait, there's no such section. Instead just gate `mockito` per-crate:

- [ ] **Step 2: `crates/athena-voice-providers/Cargo.toml` — add deps**

Add to `[dependencies]`:
```toml
reqwest = { workspace = true }
```

Add to `[dev-dependencies]`:
```toml
mockito = "1.5"
```

- [ ] **Step 3: Create `src/remote/mod.rs`**

```rust
//! Remote provider implementations (HTTP for LLM, MQTT for STT/TTS).
```

- [ ] **Step 4: Register in `lib.rs`**

Add `pub mod circuit;`, `pub mod retry;`, `pub mod remote;` — but only when the corresponding files exist. For this task, just add `pub mod remote;`.

- [ ] **Step 5: `cargo check --workspace`; commit**

---

## Task 2: Circuit breaker

- [ ] **Step 1: Create `src/circuit.rs`**

`pub struct CircuitBreaker { failure_threshold: u32, open_duration: Duration, half_open_probe_gap: Duration, state: Mutex<State> }`. `enum State { Closed { failures: u32, window_started: Instant }, Open { until: Instant }, HalfOpen { last_probe: Instant } }`.

Methods:
- `pub fn new(failure_threshold: u32, open_duration: Duration, half_open_probe_gap: Duration) -> Self`
- `pub fn can_call(&self) -> Result<(), Duration>` — Err with retry-after if open
- `pub fn record_success(&self)` — resets failures / closes circuit
- `pub fn record_failure(&self)` — increments failure count; opens circuit at threshold
- `pub fn is_open(&self) -> bool`

- [ ] **Step 2: Tests using `tokio::time::pause()` / `advance` for deterministic timing.**

Test cases:
- `closed_after_construction`
- `open_after_threshold_failures`
- `open_rejects_calls`
- `half_open_after_open_duration_probe_gap_elapses`
- `success_in_half_open_closes`
- `failure_in_half_open_reopens`

- [ ] **Step 3: Commit**

---

## Task 3: Retry decorator

- [ ] **Step 1: Create `src/retry.rs`**

```rust
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff: Vec<Duration>,  // length must match max_attempts-1
}

impl RetryConfig {
    pub fn stt_default() -> Self { /* 2 attempts, 200/500ms */ }
    pub fn llm_default() -> Self { /* 1 attempt, no retry */ }
    pub fn tts_default() -> Self { /* 2 attempts, 200/500ms */ }
}
```

Then three thin wrappers:
- `pub struct RetryingStt<S: Stt> { inner: S, config: RetryConfig, circuit: Arc<CircuitBreaker> }` implementing `Stt` by delegating `transcribe` and looping on failure.
- Same for `RetryingLlm` and `RetryingTts` (but with `max_attempts: 1` typically for LLM).

- [ ] **Step 2: Tests**

Fake providers that fail deterministically N times then succeed. Assert that the decorator recovers within `max_attempts` and gives up otherwise. Verify circuit is tripped after too many failures.

- [ ] **Step 3: Commit**

---

## Task 4: OllamaLlm

- [ ] **Step 1: Create `src/remote/ollama.rs`**

```rust
pub struct OllamaLlm {
    base_url: String,     // e.g. "http://localhost:11434"
    model: String,        // e.g. "llama3.2:3b"
    client: reqwest::Client,
    request_timeout: Duration,
}

impl OllamaLlm {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self { … }
}

#[async_trait]
impl Llm for OllamaLlm {
    async fn complete(&self, session, locale, prompt, history) -> Result<CompletionStream, BoxError> {
        // POST to {base_url}/api/chat
        // Body: {"model": self.model, "messages": [{role: "system", content: locale-prompt}, {role: "user", content: prompt}], "stream": true}
        // Parse response: line-delimited JSON, each with `{"message":{"content":"<token>"},"done":bool}`
        // Emit tokens via a stream.
    }
    fn name(&self) -> &'static str { "ollama" }
}
```

- [ ] **Step 2: Tests using `mockito`**

- `happy_path_streams_tokens` — mockito serves an SSE-like line-delimited JSON response; assert tokens accumulate to "hello world".
- `http_error_returns_llm_error_unavailable` — mockito returns 500; assert `Err`.
- `timeout_returns_llm_error_timeout` — mockito delays; assert `Err(Timeout { … })`.

- [ ] **Step 3: Commit**

---

## Task 5: MQTT-provider client (STT + TTS)

- [ ] **Step 1: Create `src/remote/mqtt_client.rs`**

```rust
pub struct MqttProviderClient {
    mqtt: rumqttc::AsyncClient,
    inbox: tokio::sync::broadcast::Receiver<rumqttc::Publish>,  // shared subscription
    request_topic_pattern: String,       // e.g. "athena/providers/stt/{name}/request"
    response_topic_pattern: String,      // e.g. "athena/providers/stt/{name}/response"
    provider_name: String,
    request_timeout: Duration,
}

impl MqttProviderClient {
    pub async fn call(&self, session: SessionId, request: serde_json::Value)
        -> Result<Vec<serde_json::Value>, BoxError>  // returns list of response messages
    { … }
}
```

Correlation: each request carries `session_id`. Responses are matched by `session_id` in the user-properties field OR JSON body.

- [ ] **Step 2: Create `src/remote/mqtt_stt.rs`**

Thin wrapper: on `transcribe`, serialise (session, locale) into a request; publish; wait for streaming responses; convert each into `Transcript`.

- [ ] **Step 3: Create `src/remote/mqtt_tts.rs`**

Same shape, but responses are binary Opus chunks.

- [ ] **Step 4: Tests**

`mqtt_client_tests.rs` — spins an in-process rumqttc broker (only reasonable place to use it), publishes a request, injects a canned response, asserts the client returns it.

- [ ] **Step 5: Commit (or split per file — three commits)**

---

## Task 6: Extend `StageChoice` + `ProviderFactory`

- [ ] **Step 1: Modify `factory.rs`**

```rust
#[serde(rename_all = "snake_case")]
pub enum StageChoice {
    Fake,
    Ollama { base_url: String, model: String },
    MqttStt { name: String },
    MqttTts { name: String },
}
```

`ProviderFactory::new` now takes `&ProviderConfig` and `Option<&MqttClient>` (since MQTT variants need the shared client). Return errors from `new` (`Result<Self, ProviderError>` — new error type) rather than silently defaulting.

- [ ] **Step 2: Update all callers** — `serve::run`, tests.

- [ ] **Step 3: Wrap every returned provider in `RetryingXxx` with the default retry config + a fresh `CircuitBreaker`.**

- [ ] **Step 4: Tests updated to include new StageChoice variants in the TOML roundtrip.**

- [ ] **Step 5: Commit**

---

## Task 7: Runtime + CLI wiring

- [ ] **Step 1: Modify `Runtime::spawn` to also expose the `AsyncClient` handle to the `ProviderFactory`.**

- [ ] **Step 2: Update `serve::run` to build the factory *after* the runtime's MQTT client exists.**

- [ ] **Step 3: Update `athena.example.toml`** with new StageChoice variants documented as commented-out alternatives:

```toml
[providers]
stt = "fake"
llm = "fake"
tts = "fake"

# Or, for real providers:
# stt = { mqtt_stt = { name = "whisper" } }
# llm = { ollama = { base_url = "http://localhost:11434", model = "llama3.2:3b" } }
# tts = { mqtt_tts = { name = "piper" } }
```

- [ ] **Step 4: Integration test — mock Ollama roundtrip.**

`crates/athena-voice-runtime/tests/ollama_integration.rs` (dev-dep `mockito`): builds a fake satellite, spawns runtime with `StageChoice::Ollama { base_url: mockito_url, model: "test" }`, drives one session, asserts the mocked Ollama response makes it to the session/done payload.

Since Plan 2 deferred the full MQTT-driven satellite test, this ollama_integration test may also be marked `#[ignore]` if the embedded broker infra is still missing. Note that clearly in commit.

- [ ] **Step 5: Commit**

---

## Task 8: Fmt + clippy + CI

- [ ] **Step 1: Run `cargo fmt --all` and `cargo clippy --workspace --all-features --all-targets -- -D warnings`. Fix any issues.**

- [ ] **Step 2: Push and verify CI green.**

- [ ] **Step 3: Commit final sweep if needed.**

---

## Definition of Done for Plan 3

Plan 3 is complete when **all** are true:

1. `cargo build --workspace` clean.
2. `cargo test --workspace` — all tests green. Target: ~100 tests (Plan 2's 89 + ~11 new).
3. `cargo clippy --workspace --all-features --all-targets -- -D warnings` — clean.
4. `cargo fmt --all --check` — clean.
5. GitHub Actions CI green on the branch.
6. `athena-voice serve --config <path>` with `[providers] llm = { ollama = { base_url = "http://localhost:11434", model = "llama3.2:3b" } }` and a locally-running Ollama container completes a session end-to-end (manual acceptance).
7. `docs/superpowers/plans/2026-07-11-athena-voice-remote-providers.md` (this file) exists and is committed.

## Explicitly deferred to later plans

- Local `whisper-rs` / `llama-cpp-rs` / `piper-rs` bindings (Plan 3.1).
- OpenAI-compatible LLM client (Plan 3.1).
- WASM host + skill loading (Plan 4).
- Dashboard (Plan 5).
