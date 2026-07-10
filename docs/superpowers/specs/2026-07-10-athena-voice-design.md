# Athena-Voice — Design Spec

**Date:** 2026-07-10
**Author:** Gekkotron (`Gekkotron`)
**Status:** Approved for implementation planning
**Predecessor:** [`github.com/Gekkotron/VoiceAssist`](https://github.com/Gekkotron/VoiceAssist) — used as a **behaviour reference only**; no code is reused (see §3).

---

## 1. Summary

Athena-Voice is an **extensible voice-assistant framework** written in Rust. It runs headless in Docker on a home Linux server, ingests audio from a custom Android satellite over MQTT, transcribes it, decides how to answer via a pluggable intent router (pattern matcher plus WASM skills, with an LLM fallback), synthesises a spoken reply, and streams it back to the phone.

The framework is domain-neutral: everything specific to home automation, French wording, Jeedom, or MQTT device topics lives outside the core. Skills are WebAssembly plugins (via Extism) and can be written in any language that targets `wasm32-wasi`. Out-of-process providers (STT, LLM, TTS, or additional skills) attach over MQTT.

---

## 2. Goals & non-goals

### Goals

- One Rust binary + a mosquitto broker are enough to run the whole system.
- Adding a skill = writing a WASM module and dropping it into a directory (no core rebuild).
- Adding a provider = subscribing to a documented MQTT topic pattern from any language.
- FR and EN are first-class locales at launch; adding a locale is a config-only change.
- End-to-end latency ≤ 1 s for skill-matched utterances, ≤ 2 s for LLM-fallback utterances, on a home LAN with a remote Ollama LLM.
- Multi-satellite, concurrent sessions.
- Persistent event log and structured errors for post-mortem.

### Non-goals (v1)

- No local microphone / `cpal`; server is headless.
- No on-device STT/TTS on Android.
- No first-party shipped skills. A dev-only smoke-test skill exists to exercise the WASM host during CI.
- No multi-tenant / multi-household routing.
- No barge-in or clarification loops.
- No local `whisper-rs` / `llama-cpp-rs` / `piper-rs` bindings if remote providers reach the Definition of Done faster — deferred to v1.1.

---

## 3. Decision matrix (locked)

| Axis | Choice | Rationale |
|---|---|---|
| Project shape | Extensible framework | Runnable core with pluggable skills |
| Runtime | **Rust** (full rewrite) | User preference; small binary, low memory, fast startup — despite ML models being the actual latency bottleneck |
| Locale support | Locale-agnostic core; FR + EN packs at v1 | Matches i18n design in the predecessor project |
| Skill plugin model | **WASM via Extism** | Language-agnostic, sandboxed, µs–ms overhead invisible against STT/LLM latency |
| Deployment | Docker home server, headless Linux | Confirmed by user |
| Satellite transport | **MQTT** (Android is another MQTT client) | Reuses the existing MQTT stack; unified with the internal bus |
| Provider strategy | Trait per stage; ship local (feature-gated) + remote (MQTT/HTTP) impls | Users can pick per-stage; local optional |
| Inter-service bus | MQTT (mosquitto in docker-compose) | Confirmed by user |
| Activation | Wake word on Android satellite | Saves bandwidth and server CPU |
| v1 shipped scope | Core pipeline end-to-end + web dashboard | Explicitly selected by user |
| Persistence | SQLite by default; Postgres via config (`sqlx`) | Zero-setup for personal use; scales if needed |
| Orchestration model | Actor-based (tokio tasks + typed mpsc channels) | Section §4 justification |
| Crate namespace | `athena-voice-*` | Avoids collisions with sibling `athena-*` projects |

---

## 4. Architecture

### 4.1 System boundaries

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     Home Linux server (Docker host)                     │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │            athena-voice   (single Rust binary, tokio)            │   │
│  │                                                                  │   │
│  │   ╭──────────────╮  ╭──────╮  ╭─────╮  ╭────────╮  ╭──────────╮  │   │
│  │   │  Satellite   │─▶│Ingest│─▶│ STT │─▶│ Intent │─▶│  Skill   │  │   │
│  │   │   Adapter    │  │+ VAD │  │Actor│  │ Router │  │Dispatcher│  │   │
│  │   │  (MQTT sub)  │  ╰──────╯  ╰─────╯  ╰────────╯  ╰─────┬────╯  │   │
│  │   ╰──────▲───────╯                                       │       │   │
│  │          │                                          ╭────▼────╮  │   │
│  │          │       ╭─────╮   ╭───────╮   ╭───────╮   │   LLM   │  │   │
│  │          ╰───────│ TTS │◀──│Response│◀─│ Skill │◀──│ (fallbk)│  │   │
│  │                  │Actor│   │ Sink   │  │Result │   ╰─────────╯  │   │
│  │                  ╰──▲──╯   ╰────────╯  ╰───────╯                │   │
│  │                     │                                           │   │
│  │            ╭────────┴─────────╮  ╭──────────╮  ╭─────────────╮  │   │
│  │            │  Provider traits │  │WASM host │  │  Event Bus  │  │   │
│  │            │  Stt / Llm / Tts │  │ (Extism) │  │ (MQTT pub)  │  │   │
│  │            │  local + remote  │  ╰─────┬────╯  ╰──────┬──────╯  │   │
│  │            ╰────────┬─────────╯        │              │         │   │
│  │                     │                  │  ╭───────────▼──────╮  │   │
│  │                     │           ╭──────▼──│ Web dashboard    │  │   │
│  │                     │           │ Skills  │ (axum + WS)      │  │   │
│  │                     │           │ (.wasm) ╰──────────────────╯  │   │
│  │                     │           ╰─────╯                         │   │
│  │                     │           ╭──────────────╮                │   │
│  │                     │           │ State store  │                │   │
│  │                     │           │ SQLite/PG    │                │   │
│  │                     │           ╰──────────────╯                │   │
│  └─────────────────────┼───────────────────────────────────────────┘   │
│                        │                                               │
│  ┌─────────────────────▼──────────┐  ┌───────────────────────────────┐  │
│  │   mosquitto MQTT broker         │  │ External provider containers │  │
│  │   (compose service)             │◀▶│  ollama / whisper / piper …  │  │
│  │   topics: athena/…              │  │  subscribed to athena/…      │  │
│  └─────────────────────────────────┘  └───────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
                              ▲
                              │  MQTT over TLS (port 8883)
              ┌───────────────┴──────────────┐
              │  Android satellite (custom)  │
              │  wake word + mic + speaker   │
              └──────────────────────────────┘
```

### 4.2 Layered responsibilities

- **Edge layer.** `SatelliteAdapter` (MQTT subscriber for `athena/sat/+/session/#`) and `Dashboard` (axum HTTP + WebSocket). Both terminate at the process boundary.
- **Pipeline layer.** Actor DAG: Ingest → VAD → STT → IntentRouter → SkillDispatcher (or LLM fallback) → TTS → ResponseSink (MQTT publish adapter) → back onto MQTT. Every actor is a `tokio::spawn`-ed task with bounded `mpsc` channels. Correlation via `SessionId` carried in every payload.
- **Extension layer.** Two surfaces:
    - **WASM plugins (Extism):** in-process skills; host functions for logging, allowlisted HTTP, per-skill KV in SQLite, scoped MQTT publish, config read.
    - **MQTT services:** any-language STT / LLM / TTS providers, or additional skills, subscribed under `athena/providers/…` and `athena/skills/…`.
- **Provider layer.** `Stt`, `Llm`, `Tts` traits with two default impls each: local (feature-gated Rust bindings — may slip to v1.1) and remote (HTTP for Ollama / OpenAI-compat; generic MQTT-provider clients).
- **Persistence layer.** `Store` trait, `sqlx` impls for SQLite (default) and Postgres. Tables: `sessions`, `transcripts`, `event_log`, `error_log`, `skill_kv`, `satellites`.
- **Observability layer.** Every actor emits a typed `Event`. Fan-out: (1) `tokio::sync::broadcast` for the dashboard, (2) MQTT mirror to `athena/events/*` for external subscribers, (3) `event_log` table for post-mortem.

### 4.3 Cross-cutting concerns

- **Config:** TOML (`athena.toml`) with env-var overrides via `figment`. Locale packs (`locales/fr.toml`, `locales/en.toml`) hold pattern rules + LLM prompt templates keyed by locale.
- **Auth:** Satellite MQTT connections use per-satellite credentials in mosquitto's password file. ACL scopes each satellite to `athena/sat/<own_sat_id>/#` publish + subscribe. TLS is mandatory for satellite connections (port 8883).
- **Deployment:** `docker/docker-compose.yml` ships `athena-voice` + `mosquitto`. Optional overlays: `docker-compose.local-ml.yml` (adds Ollama, whisper-server, piper-server), `docker-compose.pg.yml` (swaps SQLite → Postgres).
- **Tracing:** `tracing` crate; one span per session with `session_id` as a span field; JSON output on stdout captured by Docker's log driver.

---

## 5. Components

### 5.1 Workspace layout

```
athena-voice/
├── Cargo.toml                    # workspace, resolver = "2"
├── crates/
│   ├── athena-voice-core/        # type vocabulary + provider traits
│   ├── athena-voice-storage/     # sqlx Store trait + SQLite/Postgres impls
│   ├── athena-voice-providers/   # Stt/Llm/Tts adapters (local & remote)
│   ├── athena-voice-skill-sdk/   # WASM guest SDK — what skill authors import
│   ├── athena-voice-runtime/     # actors, WASM host, MQTT client, event bus, satellite adapter
│   ├── athena-voice-dashboard/   # axum HTTP + WebSocket
│   └── athena-voice-cli/         # binary — `athena-voice serve`
├── locales/                      # fr.toml, en.toml
├── skills-smoke-test/            # dev-only WASM skill for integration tests
├── migrations/                   # sqlx migrations, auto-applied at startup
├── dashboard-web/                # dashboard SPA source
├── docker/
│   ├── docker-compose.yml
│   ├── docker-compose.local-ml.yml
│   └── docker-compose.pg.yml
└── docs/
```

The binary produced by `athena-voice-cli` is named `athena-voice` (`[[bin]] name = "athena-voice"`).

### 5.2 Public surfaces

#### `athena-voice-core`

Pure types + trait definitions. No I/O.

```rust
pub struct SessionId(Uuid);
pub struct SatelliteId(String);
pub struct Locale(String);              // validated against loaded packs

pub struct AudioFrame {                 // 20 ms of s16 16 kHz mono
    pub session: SessionId,
    pub seq: u32,
    pub pcm: Bytes,
}
pub struct Transcript { pub text: String, pub is_final: bool, pub confidence: Option<f32> }
pub struct Intent { pub name: String, pub slots: BTreeMap<String, Value>, pub confidence: f32 }
pub struct Completion { pub text: String, pub finish: FinishReason }

pub enum Event {
    SessionStarted { session: SessionId, satellite: SatelliteId, locale: Locale },
    TranscriptPartial { session: SessionId, text: String },
    TranscriptFinal   { session: SessionId, text: String },
    IntentMatched     { session: SessionId, intent: Intent },
    SkillInvoked      { session: SessionId, skill: String },
    SkillPanicked     { session: SessionId, skill: String, reason: String },
    LlmFallback       { session: SessionId },
    TtsChunk          { session: SessionId, seq: u32, bytes_len: usize },
    SessionEnded      { session: SessionId, outcome: Outcome },
    ProviderError     { session: SessionId, stage: Stage, error: String },
    // … (see full enum in the implementation plan)
}

#[async_trait] pub trait Stt: Send + Sync { … }
#[async_trait] pub trait Llm: Send + Sync { … }
#[async_trait] pub trait Tts: Send + Sync { … }
```

#### `athena-voice-providers`

Modules `stt`, `llm`, `tts`. Each has:
- `local` submodule (Cargo features `stt-whisper`, `llm-llama`, `tts-piper` — off by default in v1).
- `remote` submodule with HTTP impls (Ollama, OpenAI-compat) and generic MQTT-provider clients (`athena/providers/<stage>/<name>/request` and `.../response`).
- `testing` submodule with `FakeStt`, `FakeLlm`, `FakeTts` used by higher-layer tests.
- `factory` that reads config and returns `Arc<dyn Stt>` / `Arc<dyn Llm>` / `Arc<dyn Tts>`.

#### `athena-voice-skill-sdk` (compiles to `wasm32-wasi`)

What skill authors write against:

```rust
use athena_voice_skill_sdk::{skill, Intent, HostCtx, SkillResponse, PatternRule};

#[skill(name = "weather", version = "0.1")]
pub struct Weather;

impl athena_voice_skill_sdk::Skill for Weather {
    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> { … }
    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse> {
        let city = intent.slots.get("city").and_then(|v| v.as_str()).unwrap_or("Paris");
        let temp: f64 = ctx.http_get_json(&format!("https://…/{city}"))?["temp_c"].as_f64().unwrap();
        Ok(SkillResponse::speak(format!("Il fait {temp}°C à {city}")))
    }
}
```

`HostCtx` exposes Extism host functions: `log`, `config_get`, `http_get_json` / `http_post_json` (allowlisted hosts, rate-limited), `state_get` / `state_set` (per-skill KV via `Store::skill_kv_*`), `mqtt_publish` (rate-limited, ACL'd to `athena/skills/<skill>/*`).

#### `athena-voice-runtime`

Submodules:
- `mqtt`: `rumqttc` wrapper, topic constants, publish/subscribe helpers.
- `satellite`: `SatelliteAdapter` actor (MQTT sub → `AudioFrame` stream; `TtsChunk` events → MQTT pub).
- `pipeline`: actors `Ingest`, `Vad`, `Stt`, `IntentRouter`, `SkillDispatcher`, `Llm`, `Tts`, `ResponseSink`. Each `struct` with `spawn(config, deps) -> ActorHandle`.
- `wasm`: Extism `Plugin` cache, host_fn registrations, skill registry.
- `events`: in-process broadcast + MQTT mirror task.
- `intent`: pattern-rule matcher (locale-pack rules + skill-provided rules), fuzzy match, slot extraction.

#### `athena-voice-storage`

`Store` trait: `record_session`, `finalize_session`, `append_event`, `append_error`, `save_transcript`, `skill_kv_get`, `skill_kv_set`, `provision_satellite`. Impls: `SqliteStore` (default), `PostgresStore` (behind feature). Migrations in `migrations/` are `sqlx::migrate!()`-applied at startup.

#### `athena-voice-dashboard`

Axum app:
- `GET /` → SPA.
- `GET /api/config` / `PATCH /api/config` (hot-reload).
- `GET /api/satellites`.
- `GET /api/sessions?limit=…`.
- `GET /ws` → live event stream (subscribes to `broadcast::Sender<Event>` from runtime).

#### `athena-voice-cli`

Single binary. `main` parses config, builds the actor DAG, spawns everything, waits for SIGTERM, drains, exits.

---

## 6. Data flow

### 6.1 Happy path (skill match)

1. **Wake word on Android.** App generates `session_id = uuid.v4()`. Publishes `athena/sat/<sat_id>/session/start` (QoS 1) with `{session_id, locale, started_at, sat_version}`.
2. **Server: SessionStarted.** `SatelliteAdapter` unpacks; emits `Event::SessionStarted`; `Store::record_session`; opens per-session `mpsc<AudioFrame>` (cap 64).
3. **Audio streaming.** Android streams 200 ms chunks (`~6.4 KB`, s16 16 kHz PCM) to `athena/sat/<sat_id>/session/<sid>/audio` (QoS 0). Adapter parses, pushes `AudioFrame` → Ingest → VAD → STT.
4. **STT partials.** STT streams `Transcript{is_final:false}` — mirrored to `Event::TranscriptPartial`, MQTT `athena/events/*`, and back to satellite on `athena/sat/<sat_id>/session/<sid>/transcript` (QoS 1) for live UI.
5. **Endpoint.** Android publishes `session/<sid>/end{reason:"endpoint"}` (QoS 1). Adapter drops the audio channel. STT flushes final transcript. `Event::TranscriptFinal`.
6. **Routing.** `IntentRouter` runs pattern rules (locale pack + skills). On match: `Intent{name, slots, confidence}` → `SkillDispatcher`.
7. **Skill.** `SkillDispatcher` loads WASM plugin (cached), calls `handle(intent, host_ctx)`. Response: `SkillResponse::speak(text)` (or `empty`, or `ask_llm(prompt)`).
8. **TTS.** `Tts::synthesize` streams Opus chunks. First message on `athena/sat/<sat_id>/session/<sid>/tts/meta` (QoS 1) — `{sample_rate:24000, channels:1, frame_ms:20, codec:"opus"}`. Then Opus packets on `.../tts` (QoS 0), one per MQTT message.
9. **Done.** After last TTS chunk: `athena/sat/<sat_id>/session/<sid>/done` (QoS 1) `{intent, response_text, outcome:"ok", duration_ms}`. `Event::SessionEnded`. `Store::finalize_session`. Per-session state dropped.

### 6.2 LLM-fallback path (v1 default; no skills shipped)

Steps 1–5 identical. At the router, no rule matches → `Llm::complete(prompt=text, locale, history)` streams tokens. A sentence-boundary buffer between `Llm` and `Tts` emits each sentence as soon as it closes (`.`, `!`, `?`, or a configurable soft-break length); the `Tts` actor synthesises per sentence so first Opus chunks reach the phone as soon as the first sentence completes. Emits `Event::LlmFallback` instead of `IntentMatched`.

### 6.3 Wire payloads

| Topic | Direction | Body | Notes |
|---|---|---|---|
| `athena/sat/<sat_id>/session/start` | app → server | JSON | `{session_id, locale, started_at, sat_version}` — QoS 1 |
| `athena/sat/<sat_id>/session/<sid>/audio` | app → server | binary | s16 16 kHz mono PCM chunks — QoS 0 |
| `athena/sat/<sat_id>/session/<sid>/end` | app → server | JSON | `{reason: "endpoint"\|"cancel"\|"error", client_error?}` — QoS 1 |
| `athena/sat/<sat_id>/session/<sid>/transcript` | server → app | JSON | `{is_final, text, confidence?}` — QoS 1 |
| `athena/sat/<sat_id>/session/<sid>/tts/meta` | server → app | JSON | `{sample_rate, channels, frame_ms, codec}` — QoS 1, once before first `tts` |
| `athena/sat/<sat_id>/session/<sid>/tts` | server → app | binary | one Opus packet per message — QoS 0 |
| `athena/sat/<sat_id>/session/<sid>/done` | server → app | JSON | `{intent?, response_text?, outcome, duration_ms, error?}` — QoS 1 |
| `athena/events/<kind>` | internal → external | JSON | serialized `Event` variant |
| `athena/providers/stt/<name>/request` | server → provider | JSON | `{session_id, locale, audio_ref}` (audio via companion binary topic) |
| `athena/providers/stt/<name>/response` | provider → server | JSON | streaming: `{session_id, is_final, text}` |
| `athena/providers/llm/<name>/request` | server → provider | JSON | `{session_id, locale, prompt, history}` |
| `athena/providers/llm/<name>/response` | provider → server | JSON | streaming: `{session_id, delta?, done?}` |
| `athena/providers/tts/<name>/request` | server → provider | JSON | `{session_id, locale, text, voice?}` |
| `athena/providers/tts/<name>/response` | provider → server | binary | Opus packets, `session_id` in MQTT user property |
| `athena/skills/<skill>/*` | skill → any | skill-defined | ACL: skill can only publish under its own namespace |

MQTT v5 user properties carry `session_id` on provider response topics to avoid JSON-wrapping binary bodies.

### 6.4 Cancellation

- `session/<sid>/end{reason:"cancel"}` → `SatelliteAdapter` drops the audio-frame sender.
- Every actor's future selects on a per-session `CancellationToken` (from `tokio_util::sync`).
- Provider RPCs take `&CancellationToken`; remote HTTP requests use `select!` against `token.cancelled()`.
- In-flight aborts within ~10 ms.
- `Event::SessionEnded{outcome:Cancelled}`; `session/done{outcome:"cancelled"}`.

### 6.5 Concurrent sessions

- One `SatelliteAdapter` subscribes with wildcards; routes on topic segments.
- `DashMap<SessionId, SessionState>` — each state owns: audio-frame `Sender`, `CancellationToken`, tracing span, `SatelliteId`, `opened_at`.
- Pipeline actors are shared (`Stt`, `Llm`, `Tts` are single instances). Provider impls own internal queueing (`Mutex<LlamaContext>` for llama.cpp — inherently single-threaded per model).
- Backpressure: bounded channels; when a session's channel fills, frames drop for that session only. If >10 % drops in a 3 s window → session ends `overloaded`.

### 6.6 Channel plumbing (in-process)

```
                       broadcast<Event>  (fan-out to dashboard + mqtt-events mirror task)
                                ▲
   ┌────────────────────────────┼────────────────────────────────────────────────┐
   │   SatelliteAdapter    ──── │ ─── mpsc<AudioFrame> per session (cap 64) ─▶  │
   │        │                   │                                            Vad │
   │        │                   │                                             │  │
   │        │                   │                                             ▼  │
   │        │                   │                                            Stt │
   │        │                   │                     mpsc<Transcript> ──────┤   │
   │        │                   │                     (cap 16)               ▼   │
   │        │                   │                                    IntentRouter│
   │        │                   │                                          ├─┬─┐ │
   │        │                   │             mpsc<(Session, Intent)> ─────┘ │ │ │
   │        │                   │             (cap 16)                       │ │ │
   │        │                   │                                            ▼ │ │
   │        │                   │                                  SkillDispatcher
   │        │                   │                                       (WASM host)
   │        │                   │                                            │ │ │
   │        │                   │             stream<String> ────────────────┘ │ │
   │        │                   │                                              ▼ ▼
   │        │                   │                                              Llm
   │        │                   │                                               │
   │        │                   │                                   stream<String>
   │        │                   │                                               │
   │        │                   │                                               ▼
   │        │                   │                                              Tts
   │        │                   │                                               │
   │        │                   │                                stream<OpusPacket>
   │        │                   │                                               │
   │        ▼                   │                                               │
   │   MQTT publish ◀───────────┴──────── ResponseSink (MQTT publish adapter) ◀─┘
   └─────────────────────────────────────────────────────────────────────────────┘
```

### 6.7 Latency budget (target for v1 with remote Ollama + local Piper or remote TTS)

| Stage | Target |
|---|---|
| Android publish → server receive (LAN) | < 50 ms |
| STT (streaming, Whisper small) | first partial 200–400 ms; final 400–800 ms |
| Intent match | < 5 ms |
| Skill invoke (WASM warm) | < 1 ms |
| LLM (remote Ollama 7B q4) | first token 300–800 ms; complete 1–3 s |
| TTS (streaming) | first chunk 100–200 ms after first token |
| **First TTS packet reaches phone** | **~700 ms (skill) / ~1.2 s (LLM)** |

### 6.8 Observability

Every `Event` is:
1. Written to SQLite `event_log`.
2. Broadcast on in-process `broadcast::Sender<Event>` (dashboard WS).
3. Mirrored to `athena/events/<kind>` MQTT.

`tracing` structured logs; one span per session with `session_id`; JSON stdout.

---

## 7. Error handling

### 7.1 Error taxonomy

Each crate exports typed errors via `thiserror`. `anyhow` only at the CLI boundary. Every variant:
- Knows if it's retryable (`is_retryable(&self) -> bool`).
- Knows how to describe itself to the user in the session's locale (`to_user_message(&self, &Locale) -> String`).

### 7.2 Failure matrix

Abbreviated (full failure matrix — all detection points, event names, and paired integration tests — is produced during implementation-plan authoring):

| # | Failure | Response | Satellite outcome |
|---|---|---|---|
| 1 | MQTT broker lost | Exp backoff reconnect; cancel in-flight sessions | Sessions time out |
| 2 | STT timeout | 2 retries + circuit breaker; secondary if configured; else canned phrase | `done{outcome:"error", error:"stt_unavailable"}` |
| 3 | LLM failure | No retry; secondary if configured; else canned | `done{outcome:"error", error:"llm_unavailable"}` |
| 4 | TTS failure | 2 retries; then degrade to text-only `response_text` | `done{outcome:"error", error:"tts_unavailable", response_text:"…"}` |
| 5 | WASM panic / timeout / OOM | Isolate plugin; route to LLM fallback | Transparent |
| 6 | SQLite `SQLITE_BUSY` | 3 retries with 10 / 20 / 40 ms backoff | None |
| 7 | Storage unavailable | In-memory only; bounded backlog; drop on overflow | None (observability lost) |
| 8 | Audio-frame channel full | Drop for that session; > 10 % / 3 s → end `overloaded` | `done{outcome:"overloaded"}` |
| 9 | `max_concurrent_sessions` hit | Reject new `session/start` | `done{outcome:"overloaded"}` |
| 10 | Malformed control JSON | Log warn; publish `done{error:"protocol_error"}` if session_id parseable | Fails cleanly |
| 11 | Audio chunk on unknown session | Drop; rate-limited warn | None |
| 12 | Session ID collision | Reject second `start` | `done{error:"session_collision"}` |
| 13 | VAD never endpoints | 30 s absolute timeout → force close | Ends normally |
| 14 | Sample-rate mismatch | End with `bad_audio` | Canned TTS |
| 15 | Config reload fails | Keep previous config; dashboard banner | None |

### 7.3 Retry policy

| Operation | Attempts | Backoff | Circuit? |
|---|---|---|---|
| MQTT reconnect | ∞ | exp 0.5 → 30 s cap | no |
| STT / TTS RPC | 2 | 200 / 500 ms | yes (5 failures / 30 s → open 60 s; half-open probe every 15 s) |
| LLM RPC | 1 (no retry) | — | yes |
| SQLite writes | 3 | 10 / 20 / 40 ms | no |
| Config reload | 1 | — | no |

### 7.4 Degradation ladder (per stage)

- **STT down** → primary retry → secondary → canned "je n'ai pas pu vous entendre" → session error.
- **Router match** → skill → LLM fallback → canned "je ne sais pas répondre" → session error.
- **LLM down** (fallback path) → secondary LLM → canned "mon cerveau est hors ligne" → session error.
- **TTS down** → primary retry → secondary → text-only `response_text` (no canned audio; TTS is what's broken) → session error.
- **Storage down** → in-memory; observability lost; flow uninterrupted.
- **MQTT broker down** → no degradation; `exit(1)` after 60 s of reconnect failure; Docker restart takes over.

### 7.5 Cancellation

Every actor's future:
```rust
loop {
    tokio::select! {
        _ = token.cancelled() => break,
        Some(msg) = rx.recv()  => process(msg).await,
        else => break,
    }
}
```
Provider RPCs take `&CancellationToken`. Cancel propagates to remote HTTP calls via `select!`.

### 7.6 Startup / shutdown

| Situation | Behaviour |
|---|---|
| MQTT unreachable at boot | Retry `startup_broker_timeout` (60 s); `exit(1)` |
| DB unreachable at boot | Retry `startup_db_timeout` (30 s); `exit(1)` |
| Local provider model file missing | `exit(2)` with clear error pointing to path config |
| Invalid `athena.toml` | Print all validation errors; `exit(2)` |
| No locale pack loads | `exit(2)` |
| Migrations fail | `exit(3)` |
| SIGTERM | Stop new `session/start`; wait `shutdown_grace` (10 s) for existing; hard-cancel; flush events; close DB |

### 7.7 Health endpoints

- `GET /health` — 200 while process is up (liveness).
- `GET /ready` — JSON `{"mqtt": bool, "db": bool, "providers": {"stt": "ok"|"circuit_open", …}}`. 200 if all critical (mqtt + db + ≥1 of each stage) ok; 503 otherwise.

### 7.8 Post-mortem persistence

- `event_log` (append-only): every `Event`, timestamped, indexed by `session_id`.
- `error_log`: session_id, stage, variant, message, source chain.
- `session_log`: outcome, duration, provider names, model versions.
- On restart: any session in `event_log` with `SessionStarted` but no `SessionEnded` → mark `outcome:"orphaned"` with synthetic event.
- Retention configurable; default 30 days.

---

## 8. Testing

### 8.1 Layers (fewest tests at top, most at bottom)

```
        ▲                Manual acceptance (per release, ~7 checks)
        │                End-to-end (~8 tests, testcontainers)
   fewer│                Integration (~30 tests, in-process)
        │                Unit tests (bulk)
   more▼                Property + fuzz + micro-benchmarks
```

### 8.2 Unit tests

| Crate | Key units |
|---|---|
| `athena-voice-core` | Type invariants, `Event` serde round-trip, error `is_retryable` truth table, `to_user_message` per variant × locale |
| `athena-voice-storage` | Migration idempotency, `Store` methods, `SQLITE_BUSY` retry, PG parity behind feature |
| `athena-voice-providers` | Circuit breaker state machine, retry timing (via `tokio::time::pause()`), timeout enforcement, cancellation propagation |
| `athena-voice-skill-sdk` | `#[skill]` macro codegen, `PatternRule` DSL, `HostCtx` mocks |
| `athena-voice-runtime` | VAD state machine, pattern matcher (pos + neg + slots), WASM allowlist + rate limiter, MQTT topic parser, overload detection, `CancellationToken` cascade |
| `athena-voice-dashboard` | Handler tests via `axum-test`, event stream serialization |

Runner: `cargo nextest`. Fake time via `tokio::time::pause()`. Zero real `sleep`s.

### 8.3 Provider contract tests

Every impl of `Stt` / `Llm` / `Tts` runs the same suite (in `athena-voice-providers/tests/contract/`):

```rust
async fn stt_contract<S: Stt + 'static>(stt: S) {
    // 1. happy path → non-empty final on known-good clip
    // 2. streaming emits ≥ 1 partial on > 1.5 s clip
    // 3. cancellation aborts within 200 ms
    // 4. timeout returns SttError::Timeout, no hang
    // 5. malformed audio returns SttError::BadAudio, no panic
    // 6. concurrent calls with different session_id don't interfere
    // 7. under load (100 concurrent) memory does not grow unbounded
}
```

Same shape for `Llm` (adds token ordering, empty prompt) and `Tts` (adds decodable Opus, sample-rate matches `tts/meta`). Fakes (`FakeStt` etc.) also pass — they are the default in higher-layer tests.

### 8.4 Integration tests

Full actor DAG in-process, embedded `rumqttd` broker, `:memory:` SQLite, `FakeStt`/`FakeLlm`/`FakeTts`. Test harness:

```rust
let h = TestHarness::builder()
    .with_locale("fr")
    .with_skill(load_wasm("skills-smoke-test.wasm"))
    .with_fake_stt(preset(&[("session_1", "quelle heure est-il")]))
    .build().await;

let sat = h.spawn_fake_satellite("phone-01");
let sid = sat.start_session("fr").await;
sat.stream_audio(fixture("hello_fr.pcm")).await;
sat.end_session(sid).await;

assert_event_sequence!(h.events(sid).await, [
    SessionStarted { .. },
    TranscriptPartial { .. },
    TranscriptFinal { text: "quelle heure est-il", .. },
    IntentMatched { intent: { name: "time.query", .. } },
    SkillInvoked { skill: "smoke", .. },
    TtsChunk { .. }, TtsChunk { .. },
    SessionEnded { outcome: Ok, .. },
]);
```

Golden event streams via `insta` snapshots.

**Every row of the failure matrix (§7.2) has a paired integration test.** Adding a row = adding a test in the same PR.

### 8.5 End-to-end tests

`testcontainers-rs` runs the actual production binary + `mosquitto:2` alongside. A fake satellite is a `rumqttc` client. Suite:

1. Skill happy path (smoke-test WASM skill in FR).
2. LLM fallback (no skill loaded).
3. Cancellation mid-utterance.
4. Concurrent 3 sessions from 3 satellites.
5. Overload (11th session rejected when limit = 10).
6. Broker restart mid-session → session errored; next session works.
7. Server restart with in-flight session → session marked orphaned.
8. Config hot-reload.

### 8.6 Load / soak / chaos

- **Load:** 100 concurrent sessions × 5 min. Pass = p99 first-TTS latency < 2× p50, no unbounded task growth.
- **Soak:** 10 concurrent × 60 min. Pass = RSS ± 10 % after warmup, no `SessionOverloaded`.
- **Chaos:** `toxiproxy` in front of fake providers — 100 ms latency, 5 % loss, full drop, recover. Pass = circuit-open transitions clean; no session hangs beyond its timeout.

Nightly in CI, not per PR.

### 8.7 Manual acceptance (per release)

Real Android device on home network. Checklist recorded on video:

1. Wake word, FR: "quelle heure est-il" → LLM answers.
2. Wake word, EN: "what time is it" → LLM answers.
3. Push-to-talk long (10 s) → transcript + response.
4. Cancel mid-utterance → ends < 500 ms, no TTS.
5. Two satellites concurrently → both get responses, no cross-talk.
6. Network drop mid-session → graceful error; next session works.
7. `docker compose restart athena-voice` → satellite auto-reconnects.

### 8.8 Property, fuzz, benchmarks

- **`proptest`**: pattern-rule DSL parser, MQTT topic parser (no-panic-on-any-input + valid-round-trips), Opus framing, config schema.
- **`cargo-fuzz`**: wire deserializers on control topics.
- **`criterion`**: intent matcher (> 100 k rules/s scan), Opus encode step (< 5 ms per 20 ms frame), WASM invoke overhead (< 500 µs cached).

### 8.9 Coverage

| Layer | Target |
|---|---|
| `athena-voice-core` | ≥ 90 % |
| `athena-voice-storage` | ≥ 80 % |
| `athena-voice-providers` | ≥ 80 % |
| `athena-voice-skill-sdk` | ≥ 85 % |
| `athena-voice-runtime` | ≥ 70 % |
| `athena-voice-dashboard` | smoke-tested |
| `athena-voice-cli` | smoke-tested |

Via `cargo llvm-cov`. Codecov gate: CI fails on regression > 2 %.

### 8.10 CI (`.github/workflows/`)

- **`ci.yml`** — on PR + main push:
  1. `cargo fmt --check`
  2. `cargo clippy --workspace --all-features -- -D warnings`
  3. `cargo deny check`
  4. `cargo nextest run --workspace`
  5. `cargo llvm-cov` → Codecov
  6. Feature matrix: `--no-default-features`, `--features stt-whisper,tts-piper`, `--all-features` (Linux); `--no-default-features` on macOS
- **`e2e.yml`** — nightly + on release tag: full E2E + load/soak.
- **`docker.yml`** — multi-arch image (`linux/amd64` + `linux/arm64`) on tag → GHCR.
- **`release.yml`** — GitHub Release with binaries + Docker digest + changelog.

---

## 9. Definition of Done for v1

The framework ships when **all** are true:

1. **Green pipeline** on Linux amd64 + arm64. Coverage targets met.
2. **Full pipeline end-to-end.** A recorded French utterance from a fake satellite yields an Opus TTS response over MQTT in a `testcontainers` stack, reproducible with `docker compose`.
3. **Two providers per stage.** `FakeStt/Llm/Tts` + one real impl per stage (either local via features, or MQTT-provider client — decision at implementation-plan time).
4. **Locale packs load.** FR + EN packs load, validated at startup; drive both pattern matcher and canned error phrases.
5. **WASM host works.** Smoke-test skill runs, exercises every host function; behaviour covered in the integration suite. Not shipped to end users.
6. **Dashboard works.** `/` renders, `/ws` streams live events, `/api/config` reloads cleanly.
7. **Manual acceptance checklist passes** against a real Android satellite.
8. **Docs cover:** `docker compose up` run book, `docs/skills/writing-a-skill.md`, `docs/locales/adding-a-locale.md`, `docs/providers/adding-a-provider.md`.

---

## 10. Deferred to post-v1

- Local `whisper-rs` / `llama-cpp-rs` / `piper-rs` bindings if remote providers reach Done faster.
- First-party skills (weather, timers, home-automation adapter for Jeedom / Home Assistant / etc.).
- Multi-tenant / multi-household routing.
- Barge-in / interruption during TTS playback.
- Voice authentication (per-user voiceprint).
- Server-side wake-word (satellite currently owns it).
- Dashboard: user-facing skill install / config UI (v1 dashboard is read-mostly).

---

## 11. Open questions (to resolve before or during implementation)

1. **Dashboard SPA stack.** SvelteKit vs plain HTMX + Alpine. Deferred to the implementation plan; either works and neither affects the runtime crates.
2. **Opus bitrate & complexity** for TTS. Sensible defaults exist (24 kHz, 32 kbit/s, complexity 5); confirm with real network measurements before v1 release.
3. **Whisper model choice** if local STT ships in v1: `small.en` for EN, `small` for multilingual — or push to v1.1 and use a remote whisper-server container as the only STT option.
4. **Ollama vs OpenAI-compat as the primary remote LLM.** Ollama is likely; leaving both HTTP impls in the box so users can pick.
5. **Configuration hot-reload scope.** Everything is reloadable in principle; some (provider swap) requires draining sessions. Draw the line in the implementation plan.

---

*End of design spec.*
