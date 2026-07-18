# Plan

Currently executing **Plan 4 — WASM skill system**. Full task text lives at
`docs/superpowers/plans/2026-07-13-athena-voice-skill-system.md`; each task
below references the plan by number and gives the worker the acceptance
criterion in one line so it can pick up without further prompting. Tasks
1–4, 7, and 9 are already merged.

## Backlog

- [ ] Task A — Barge-in on new final transcript
      Add `Event::BargeIn { session, reason: BargeInReason }` (`NewFinalTranscript`, `VadSpeechStart` reserved) and `Event::SkillCancelled { session, skill }` to `athena-voice-core/src/event.rs`.
      In `pipeline/router.rs`, keep a per-session `utterance_epoch: u64`; bump on every final transcript. Snapshot the epoch before awaiting `dispatcher.call(...)`; if it has moved on when the call resolves, emit `Event::SkillCancelled` and drop the `SkillResponse::Speak/AskLlm` instead of forwarding to TTS/LLM.
      Emit `Event::BargeIn { reason: NewFinalTranscript }` whenever a final transcript arrives while a prior utterance's dispatch or TTS is still in flight (epoch > 0 with pending work).
      `pipeline/tts.rs` (and `sink.rs` if it buffers) subscribe to `Event::BargeIn` and flush queued/streaming speech tokens so the previous response stops playing.
      Add a `CancellationToken` per dispatch so the awaiting side of `SkillDispatcherHandle::call` can bail out immediately; the `spawn_blocking` WASM task finishes naturally and its result is dropped (Extism can't be interrupted mid-call).
      Tests in `pipeline/router.rs`: (1) two rapid final transcripts — first dispatch's speech is dropped, only second reaches `tts_tok_tx`, `Event::SkillCancelled` + `Event::BargeIn` observed; (2) single final transcript — no `BargeIn`/`SkillCancelled` (regression); (3) a mock TTS observing `Event::BargeIn` flushes its buffer.
      Deferred (do NOT scope here): VAD-driven barge-in on `VadSpeechStart`. That requires a real VAD upgrade and lands in a separate task.
      Success criteria: `cargo nextest run --workspace` green including the 3 new tests; `cargo clippy --workspace --all-features --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean.




## In progress

## Done

- [x] Plan 4 Task 12 — Config + docs + CI
      Add `[skills]` section to `athena.toml`: `dir = "/etc/athena-voice/skills"` (empty by default). Per-skill config: `[skills.<name>] http_allowlist = [ … ] config = { … }`.
      Update `athena.example.toml`, run `cargo fmt --all`, run `cargo clippy --workspace --all-features --all-targets -- -D warnings`.
      Update CI (GitHub Actions) to install the `wasm32-wasip1` target since the smoke-test crate is built there.
      Push + verify CI green.
      Success criteria: Task 12 in the Plan 4 doc satisfied; CI green on the resulting PR.

- [x] Plan 4 Task 11 — Integration test: end-to-end skill dispatch
      Add `crates/athena-voice-runtime/tests/skill_dispatch.rs`: spawn runtime with the smoke-test .wasm; simulate a final transcript matching the FR pattern; assert `Event::IntentMatched` + `Event::SkillInvoked` + a TTS chunk carrying the expected speech; assert no `Event::LlmFallback`.
      Success criteria: `cargo nextest run --workspace` green including the new integration test.

- [x] Plan 4 Task 10 — `skills-smoke-test` WASM skill
      New `skills-smoke-test/` crate (excluded from workspace; targets `wasm32-wasip1` with `[lib] crate-type = ["cdylib"]`; depends on `athena-voice-skill-sdk`).
      `src/lib.rs`: implements `Skill`, returns one FR pattern `"quelle heure est-il"` → intent `time.query`. `handle` calls every host function (log, config_get, state_set/get, mqtt_publish, http_get_json with a mocked-in-test allowed host), then returns `SkillResponse::Speak("il est … heure")`.
      Build: `cargo build --target wasm32-wasip1 --manifest-path skills-smoke-test/Cargo.toml`. Rebuild in a `build.rs` and load from `CARGO_TARGET_DIR` (preferred over committing the .wasm).
      Success criteria: Task 10 in the Plan 4 doc satisfied.

- [x] Plan 4 Task 8 — WASM host + guest ABI
      Wire Task 3's `HostCtx` methods through `extism_pdk::host_fn` bindings on the guest side; compile the SDK to `wasm32-wasip1` in a smoke build.
      Host-side `SkillDispatcher` (`wasm/dispatcher.rs`) is a tokio actor that receives `(session_id, intent)` and calls `SkillRegistry::dispatch`, emitting `Event::SkillInvoked`/`Event::SkillPanicked`.
      Extism invocation is CPU-bound and blocking — dispatcher uses `tokio::task::spawn_blocking`.
      Success criteria: Task 8 in the Plan 4 doc satisfied; workspace tests + clippy green; guest SDK compiles to `wasm32-wasip1`.

- [x] Plan 4 Task 6 — Skill registry + loader in `wasm/registry.rs`
      Implement `SkillRegistry { plugins, patterns }` with `load_dir(dir, deps)` that iterates `*.wasm` files, loads each with Extism, calls the exported `pattern_rules` fn to populate the pattern index.
      Add `SkillRegistry::dispatch(skill, intent) -> Result<SkillResponse, SkillError>`.
      Tests: fixture wasm file OR mocked plugin.
      Success criteria: Task 6 in the Plan 4 doc satisfied; workspace tests + clippy green.

- [x] Plan 4 Task 5 — Host functions in `wasm/host_fns.rs`
      Register Extism host functions for `host_log`, `host_config_get`,
      `host_state_get/set` (via `athena_voice_storage::Store`),
      `host_mqtt_publish` (ACL: topic must start with `athena/skills/<skill_name>/`),
      and `host_http_get_json` (allowlist per skill).
      Each takes a `UserData` payload carrying the skill's name, store,
      mqtt client, allowlist, and config.
      Tests: mock the WASM plugin side, verify host-fn dispatch AND ACL/allowlist enforcement.
      Success criteria: full task text in `docs/superpowers/plans/2026-07-13-athena-voice-skill-system.md` Task 5 satisfied; `cargo nextest run --workspace` green; `cargo clippy --workspace --all-features -- -D warnings` clean.
