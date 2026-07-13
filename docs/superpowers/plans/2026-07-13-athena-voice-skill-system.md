# Athena-Voice — Plan 4: WASM Skill System

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the `IntentRouter` from an unconditional LLM-fallback into a real router: on each final transcript, run the pattern matcher; if a rule matches, dispatch to the corresponding WASM skill via Extism; otherwise fall through to the LLM as before. Ship an `athena-voice-skill-sdk` guest crate (skill authors' surface), an Extism host in `athena-voice-runtime`, host functions (`log`, `config_get`, `http_get_json`, `state_get/set`, `mqtt_publish`), a pattern-matcher engine (JSON-DSL rules with fuzzy match + slot extraction), and a `skills-smoke-test` dev-only WASM skill that exercises every host function.

**Architecture:** New guest crate `athena-voice-skill-sdk` (target `wasm32-wasip1`) exports the `Skill` trait, generates the Extism ABI via a `#[skill]` proc macro, and stubs `HostCtx` for host-function calls. Host-side `athena-voice-runtime` gains `wasm/` module: Extism `Plugin` cache, `SkillRegistry`, host_fn implementations. Pattern matcher lives in `athena-voice-runtime/src/intent/` — pure Rust, no WASM required. Skills configuration goes into `[skills]` section of `athena.toml`.

**Tech Stack:** `extism` 1.x (WASM host runtime), `sonic-rs` or `serde_json` for pattern matcher, `strsim` for fuzzy match, `wasm32-wasip1` target (added via rustup by CI).

## Global Constraints

- **Extism** is the WASM ABI. Not `wasmtime` directly (too low-level for this scope) or `wasi-component`.
- **No breaking changes to Plans 1–3 public APIs.** Additive only.
- **Skills are optional.** With zero .wasm files in the skill dir, the runtime behaves exactly like Plan 3 (always LLM fallback).
- **Host functions are minimal in Plan 4.** `log`, `config_get`, `state_get/set`, `mqtt_publish`, `http_get_json` (allowlisted hosts only). Auth, streaming HTTP, and richer state APIs land in later plans.
- **`skills-smoke-test`** is a dev-only crate under `skills-smoke-test/` (workspace root, NOT under `crates/`). It's cross-compiled to `wasm32-wasip1` via a build script in CI, but not shipped as a runtime artifact.
- **Pattern matcher** is JSON-DSL-driven, not code-generated. Skills return a `Vec<PatternRule>` for a given locale.
- **Same edition/toolchain/attribution rules as prior plans.**

## File structure produced by this plan

```
athena-voice/
├── Cargo.toml                                            # (mod) new workspace member + deps
├── athena.example.toml                                    # (mod) [skills] section documented
├── skills-smoke-test/                                     # (new) dev-only WASM skill
│   ├── Cargo.toml                                        # wasm32-wasip1 target, cdylib
│   └── src/lib.rs                                        # exercises every host fn
├── crates/
│   ├── athena-voice-skill-sdk/                            # (new) guest side
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                                    # re-exports
│   │       ├── skill.rs                                  # Skill trait + Intent + PatternRule
│   │       ├── host.rs                                   # HostCtx (extism_pdk wrappers)
│   │       └── response.rs                               # SkillResponse variants
│   └── athena-voice-runtime/                              # (mod) host side gains WASM host
│       ├── Cargo.toml                                    # (mod) extism dep
│       └── src/
│           ├── intent/                                   # (new) pattern matcher
│           │   ├── mod.rs
│           │   ├── rule.rs                               # PatternRule host-side type
│           │   ├── engine.rs                             # match logic + slot extraction
│           │   └── loader.rs                             # locale-pack rule import
│           ├── wasm/                                     # (new) Extism host
│           │   ├── mod.rs
│           │   ├── registry.rs                           # SkillRegistry (loads *.wasm)
│           │   ├── host_fns.rs                           # log/config_get/state/mqtt/http
│           │   └── dispatcher.rs                         # SkillDispatcher actor
│           └── pipeline/router.rs                        # (mod) uses matcher + dispatcher
```

---

## Task 1: Workspace additions + `athena-voice-skill-sdk` skeleton

- [ ] Root `Cargo.toml`: add `skills-smoke-test` and `athena-voice-skill-sdk` to workspace members. Add `extism = "1"`, `extism-pdk = "1"`, `strsim = "0.11"` to workspace.dependencies.
- [ ] Create `crates/athena-voice-skill-sdk/Cargo.toml` (regular Rust crate, exports types host + guest share).
- [ ] Create `crates/athena-voice-skill-sdk/src/lib.rs` with empty `pub mod skill; pub mod host; pub mod response;` stubs.
- [ ] `cargo check --workspace`; commit.

## Task 2: Skill SDK core types

- [ ] `skill.rs`: `pub trait Skill { fn name(&self) -> &str; fn pattern_rules(&self, locale: &str) -> Vec<PatternRule>; fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError>; }`. `pub struct Intent { name: String, slots: BTreeMap<String, serde_json::Value> }`. `pub struct PatternRule { intent: String, phrases: Vec<String>, slots: Vec<SlotSpec> }`. `pub struct SlotSpec { name: String, kind: SlotKind }`.
- [ ] `response.rs`: `pub enum SkillResponse { Speak(String), Empty, AskLlm(String) }`. `pub enum SkillError { HttpFailed(String), MqttFailed(String), Custom(String) }`.
- [ ] Tests: type-serde roundtrips.
- [ ] Commit.

## Task 3: HostCtx stubs

- [ ] `host.rs`: `pub struct HostCtx { … }` with methods `log(level, msg)`, `config_get(key) -> Option<String>`, `state_get(key) -> Option<Vec<u8>>`, `state_set(key, val)`, `mqtt_publish(topic, payload)`, `http_get_json(url) -> Result<serde_json::Value, SkillError>`.
- [ ] In this task, the methods are `unimplemented!` stubs — the real Extism guest-side calls come in Task 8 when we wire the guest ABI. Host-side implementations live in Task 5.
- [ ] Commit.

## Task 4: Runtime WASM host — `wasm/` module skeleton + Extism dep

- [ ] Add `extism = "1"` to `athena-voice-runtime/Cargo.toml`.
- [ ] Create `athena-voice-runtime/src/wasm/{mod.rs,registry.rs,host_fns.rs,dispatcher.rs}` — stubs only.
- [ ] `pub mod wasm;` in `lib.rs`.
- [ ] `cargo check --workspace`; commit.

## Task 5: Host functions

- [ ] `wasm/host_fns.rs`: register Extism host functions:
    - `host_log(plugin: &mut CurrentPlugin, params, results) -> Result<()>` — logs via `tracing`.
    - `host_config_get(…)` — reads from a `HashMap<String, String>` bound to the skill.
    - `host_state_get/set(…)` — uses `athena_voice_storage::Store::skill_kv_get/set` (needs `Arc<dyn Store>` bound per skill).
    - `host_mqtt_publish(…)` — takes a topic + bytes; ACL: must start with `athena/skills/<skill_name>/`.
    - `host_http_get_json(…)` — uses `reqwest`; allowlisted hosts only (config-driven per skill).
- [ ] All host_fns take a `UserData` payload that holds the skill's context (name, store, mqtt client, allowlist, config).
- [ ] Tests: mock the WASM plugin side, verify host fn dispatch and ACL enforcement.
- [ ] Commit.

## Task 6: Skill registry + loader

- [ ] `wasm/registry.rs`: `SkillRegistry { plugins: HashMap<String, Arc<Mutex<extism::Plugin>>>, patterns: HashMap<String, Vec<PatternRule>> }`.
- [ ] `SkillRegistry::load_dir(dir: &Path, deps: SkillDeps) -> Result<Self, RuntimeError>` — iterates `*.wasm` files, loads each with Extism, calls the exported `pattern_rules` fn to populate the pattern index.
- [ ] `SkillRegistry::dispatch(skill: &str, intent: Intent) -> Result<SkillResponse, SkillError>`.
- [ ] Tests: fixture wasm file OR mocked plugin.
- [ ] Commit.

## Task 7: Pattern matcher engine

- [ ] `intent/rule.rs`: host-side `PatternRule` mirrors the SDK type (serde-compatible).
- [ ] `intent/engine.rs`: `IntentMatcher::match(text: &str, locale: Locale) -> Option<Intent>`. Tokenises `text`, iterates loaded rules for the locale, computes fuzzy similarity per phrase (via `strsim::normalized_damerau_levenshtein`), picks the best match above threshold (0.8). Extracts slots by finding the phrase pattern with placeholders (e.g., `"météo à {city}"` → `{city: "Paris"}` when input is `"météo à Paris"`).
- [ ] `intent/loader.rs`: aggregate rules from all loaded skills into a `HashMap<Locale, Vec<(PatternRule, skill_name)>>`.
- [ ] Tests: exact match; fuzzy match near threshold; slot extraction; multiple rules; ambiguous rules pick highest score.
- [ ] Commit.

## Task 8: WASM host + guest ABI

- [ ] Guest SDK Task 3's `HostCtx` methods now use `extism_pdk::host_fn` bindings to call the host. Compile the SDK to `wasm32-wasip1` in a smoke build.
- [ ] Host-side `SkillDispatcher` (in `wasm/dispatcher.rs`) is a tokio actor that receives `(session_id, intent)` and calls `SkillRegistry::dispatch`, emitting `Event::SkillInvoked` / `Event::SkillPanicked` / etc.
- [ ] Extism plugin invocation is CPU-bound and blocking — dispatcher uses `tokio::task::spawn_blocking`.
- [ ] Commit.

## Task 9: Router integration

- [ ] `pipeline/router.rs`: extend to accept `Arc<IntentMatcher>` and `mpsc::Sender<(Intent, String)>` (intent, skill_name). On each `TranscriptFinal`:
    - Run matcher. If match found → publish to skill dispatcher channel + emit `Event::IntentMatched`.
    - Else → LLM fallback (existing behaviour) + `Event::LlmFallback`.
- [ ] `pipeline/mod.rs`: spawn `SkillDispatcher`. Wire router → dispatcher → response back into TTS.
- [ ] Runtime::spawn: build `SkillRegistry` from config, plumb through.
- [ ] Tests: with no skills loaded, router still LLM-fallbacks (regression). With smoke-test skill loaded + a rule matching, router dispatches to skill instead of LLM.
- [ ] Commit.

## Task 10: `skills-smoke-test` WASM skill

- [ ] `skills-smoke-test/Cargo.toml`: `[lib] crate-type = ["cdylib"]`, `edition = "2024"`, targets `wasm32-wasip1`. Depends on `athena-voice-skill-sdk`.
- [ ] `src/lib.rs`: implements `Skill`, returns one FR pattern `"quelle heure est-il"` → intent `time.query`. In `handle`, calls every host function (log, config_get, state_set/get, mqtt_publish, http_get_json with a mocked-in-test allowed host), then returns `SkillResponse::Speak("il est … heure")`.
- [ ] `cargo build --target wasm32-wasip1 --manifest-path skills-smoke-test/Cargo.toml`; commit the .wasm as a fixture under `crates/athena-voice-runtime/tests/fixtures/smoke.wasm` OR (better) rebuild in a build.rs and load from CARGO_TARGET_DIR.
- [ ] Commit.

## Task 11: Integration test — end-to-end skill dispatch

- [ ] `crates/athena-voice-runtime/tests/skill_dispatch.rs`: spawn runtime with the smoke-test .wasm; simulate a final transcript matching the FR pattern; assert `Event::IntentMatched` + `Event::SkillInvoked` + a TTS chunk carrying the expected speech; assert no `Event::LlmFallback`.
- [ ] Commit.

## Task 12: Config + docs + CI

- [ ] `[skills]` section in `athena.toml`: `dir = "/etc/athena-voice/skills"` (empty by default). Per-skill config: `[skills.<name>] http_allowlist = [ … ] config = { … }`.
- [ ] Update `athena.example.toml`.
- [ ] `cargo fmt --all` + `cargo clippy --workspace --all-features --all-targets -- -D warnings`.
- [ ] Push + verify CI green (need to add a step that installs the `wasm32-wasip1` target if the smoke-test crate is built in CI).
- [ ] Commit final sweep.

---

## Definition of Done for Plan 4

1. `cargo build --workspace` clean.
2. `cargo test --workspace` — all tests green. Target: ~115 tests (Plan 3's 101 + ~14 new).
3. `cargo clippy --workspace --all-features --all-targets -- -D warnings` — clean.
4. `cargo fmt --all --check` — clean.
5. GitHub Actions CI green.
6. `skills-smoke-test.wasm` builds via `cargo build --target wasm32-wasip1`.
7. Integration test drives a final transcript through the router, gets a skill dispatch (not LLM fallback), receives the expected TTS.
8. With zero .wasm files configured, runtime behaves exactly like Plan 3 (regression pinned by a test).

## Explicitly deferred to later plans

- Streaming HTTP inside `http_get_json` (multiline / SSE) — Plan 4.1.
- Per-skill auth / signed WASM — Plan 5+.
- Barge-in / interruption while a skill is running — Plan 5+.
- Skill hot-reload without restart — Plan 5+.
- WASM Component Model / WASI Preview 2 — future.
