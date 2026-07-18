# Plan

Currently executing **Plan 4 — WASM skill system**. Full task text lives at
`docs/superpowers/plans/2026-07-13-athena-voice-skill-system.md`; each task
below references the plan by number and gives the worker the acceptance
criterion in one line so it can pick up without further prompting. Tasks
1–4, 7, and 9 are already merged.

## Backlog





- [ ] Task E — Weather skill (Open-Meteo)
      New crate at repo root `skills-weather/` (same layout as `skills-home/`/`skills-timer/`: `crate-type = ["cdylib"]`, edition 2024, Gekkotron author, NOT in workspace, release LTO/opt-z/strip).
      Provider: Open-Meteo — no API key required. Two endpoints: geocoding `https://geocoding-api.open-meteo.com/v1/search?name={city}&language=fr&count=1`; forecast `https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min,weather_code&timezone=auto`.
      Per-skill config in `[skills.weather]`: `http_allowlist = ["geocoding-api.open-meteo.com", "api.open-meteo.com"]`; `config = { default_city = "Paris", units = "celsius" }`. Add a commented `[skills.weather]` example to `athena.example.toml`.
      FR patterns (EN deferred): `"quel temps fait-il"` → `weather.now` (no slots — uses `default_city`); `"quel temps fait-il à {city}"` → `weather.now`, slot `city: SlotKind::String`; `"météo à {city}"` → `weather.now`; `"quel temps fera-t-il demain"` → `weather.tomorrow`; `"quel temps fera-t-il demain à {city}"` → `weather.tomorrow`.
      Handler logic in `skills-weather/src/lib.rs`: (1) resolve city: slot > `config_get("default_city")` > hard-coded `"Paris"`. (2) Cache geocoding via `state_get/set("geo/{city_lowercase}")` (compact `{lat, lon, name}` JSON; TTL/refresh is a Plan 5 concern — call out in code comment). (3) Call forecast; parse `current.temperature_2m` + `current.weather_code` for `weather.now`; parse `daily[0]` for `weather.tomorrow`. (4) `weather_code` → FR phrase via `weather_code.rs` module (all WMO codes → short FR: `0 => "temps clair"`, `1..=3 => "quelques nuages"`, `61..=65 => "de la pluie"`, `71..=75 => "de la neige"`, `95..=99 => "un orage"`; unknown code → `"un temps particulier"`; unit-tested for every documented code range). (5) Respond `SkillResponse::Speak("il fait <temp> degrés à <name>, <phrase>")` for `weather.now`; `SkillResponse::Speak("demain à <name>, il fera entre <min> et <max> degrés avec <phrase>")` for `weather.tomorrow`.
      Error paths: geocoding returns empty results → `SkillResponse::Speak("désolé, je ne trouve pas <city>")` and NO forecast call; HTTP error → `SkillResponse::Speak("désolé, le service météo est indisponible")`; JSON parse error → same message + `log("error", ...)`.
      Integration test `crates/athena-voice-runtime/tests/weather_end_to_end.rs`: add `wiremock` to workspace dev-deps; spin up two mock endpoints on deterministic ports; per-skill `http_allowlist = ["127.0.0.1"]`; skill reads a `base_url` from `config_get` (default = the real endpoints) so tests can override to the wiremock host. Cases: (1) `"quel temps fait-il à Lyon"` — geocoding returns `Lyon (45.75, 4.85)`, forecast returns `temp=18.0, weather_code=1` → TTS contains `"il fait 18 degrés à Lyon, quelques nuages"`. (2) `"quel temps fera-t-il demain"` (default_city=Paris) — geocoding returns Paris, forecast daily returns `min=8, max=15, weather_code=61` → TTS contains `"demain à Paris, il fera entre 8 et 15 degrés avec de la pluie"`. (3) `"quel temps fait-il à Zzzz"` — geocoding returns empty results → TTS `"désolé, je ne trouve pas Zzzz"` AND wiremock forecast endpoint `.expect(0)` (never called).
      Success criteria: `cargo nextest run --workspace` green including all weather_code unit tests + 3 integration cases; smoke-test/timer/home integration tests still green (regression pin); clippy + fmt clean; `skills-weather.wasm` builds via `cargo build --target wasm32-wasip1 --manifest-path skills-weather/Cargo.toml`; `athena.example.toml` shows `[skills.weather]` example.


## In progress

## Done

- [x] Task D — Home automation (MQTT) skill
      Extend `SkillConfig` in `wasm/registry.rs` with `pub mqtt_publish_allowlist: Vec<String>` (glob-style MQTT prefixes: `home/salon/light/set`, `home/+/light/set`, `home/#`). Default empty. Plumb through `SkillCtx` into `host_fns.rs`.
      Broaden `host_mqtt_publish` topic check: allow iff topic matches built-in `athena/skills/<name>/*` OR any prefix in `mqtt_publish_allowlist`. Implement `mqtt_topic_matches(pattern, topic)` with real MQTT wildcard semantics (`+` matches one level, `#` matches the tail). Unit tests: `home/+/light/set` matches `home/salon/light/set` but not `home/salon/kitchen/light/set`; `home/#` matches any tail; empty allowlist leaves the default ACL untouched (regression pin for smoke test).
      Update `athena.example.toml` to document `mqtt_publish_allowlist` under a per-skill section, with a commented `[skills.home]` example.
      New crate at repo root `skills-home/` (mirrors `skills-timer/`/`skills-smoke-test/`: `crate-type = ["cdylib"]`, edition 2024, Gekkotron author, NOT in workspace, release profile with LTO/opt-z/strip).
      Entities declared as one stringified JSON value in `[skills.home] config = { entities = "…" }` (since `config` is `HashMap<String, String>`). Schema: `[{ "name": "lumière du salon", "room": "salon", "kind": "light|switch", "set_topic": "home/salon/light/set", "on_payload": "ON", "off_payload": "OFF" }, …]`. The skill parses it lazily on first `handle` and caches in a `static OnceCell`.
      FR patterns (EN deferred): `"allume la lumière du {room}"` → `home.light.on`; `"éteins la lumière du {room}"` → `home.light.off`; `"allume {device}"` → `home.device.on`; `"éteins {device}"` → `home.device.off`. Slot kinds: `SlotKind::String`.
      Resolution: for `home.light.{on,off}` — find entity with `kind == "light" && room == slot.room`; for `home.device.{on,off}` — find entity whose `name` fuzzy-matches `slot.device` via `strsim::normalized_damerau_levenshtein ≥ 0.75` (SDK re-exports it — worker confirms). Multiple matches → highest similarity wins; log all considered names at debug level.
      Resolved match: publish `entity.set_topic` with `entity.on_payload`/`entity.off_payload`; respond `SkillResponse::Speak("d'accord")`. Unknown entity: NO publish + `SkillResponse::Speak("désolé, je ne connais pas <name>")`. Publish error: `SkillResponse::Speak("je n'ai pas pu envoyer la commande")`.
      Integration test `crates/athena-voice-runtime/tests/home_end_to_end.rs`: in-memory `SqliteStore` + `skills-home.wasm` + per-skill config with entities (`lumière du salon` → `home/salon/light/set`, `prise du bureau` → `home/bureau/switch/set`); fake MQTT client capturing publishes; per-skill `mqtt_publish_allowlist = ["home/+/light/set", "home/+/switch/set"]`. Cases: (1) `"allume la lumière du salon"` → `home/salon/light/set: ON` captured + TTS `"d'accord"`; (2) `"éteins la prise du bureau"` → `home/bureau/switch/set: OFF` captured + TTS `"d'accord"`; (3) `"allume la lumière de la piscine"` → NO publish + TTS `"désolé"`.
      Add a `host_fns.rs` test proving the smoke-test skill still can't publish to `home/#` when its allowlist is empty (permission regression).
      Success criteria: `cargo nextest run --workspace` green including 3 new host_fn wildcard tests + 3 integration cases + regression pin; smoke-test integration still green; clippy + fmt clean; `skills-home.wasm` builds via `cargo build --target wasm32-wasip1 --manifest-path skills-home/Cargo.toml`.

- [x] Task C — Timer / reminder skill + host scheduler
      New storage migration `0002_scheduled_events.sql`: `scheduled_events(id INTEGER PRIMARY KEY, skill TEXT NOT NULL, fires_at_ms INTEGER NOT NULL, mqtt_topic TEXT NOT NULL, payload BLOB NOT NULL, created_at_ms INTEGER NOT NULL)`, index on `(fires_at_ms)`.
      Extend `Store` with `schedule_event(skill, fires_at_ms, topic, payload) -> i64`, `pop_due_events(now_ms) -> Vec<ScheduledEvent>` (transactional SELECT + DELETE by id), `delete_scheduled(id)`. Full test coverage (empty, single-due, multiple-due, delete-by-id).
      New host function `host_schedule_mqtt(fires_at_ms: i64, topic: String, payload: Vec<u8>) -> Result<i64, SkillError>`. Same ACL as `host_mqtt_publish` (topic must start with `athena/skills/<skill_name>/`). Add `HostCtx::schedule_mqtt` in the guest SDK (Extism binding + `for_testing` stub). Extend the ABI header comment in `wasm/registry.rs`.
      Add `Event::ScheduledFired { skill, id }` to `athena-voice-core/src/event.rs`.
      New `wasm/scheduler.rs`: `SchedulerTask` — tokio task that ticks every 1 s, calls `pop_due_events(Utc::now().timestamp_millis())`, publishes each event via the MQTT client, emits `Event::ScheduledFired`.
      New crate at repo root `skills-timer/` (mirrors `skills-smoke-test/`: `crate-type = ["cdylib"]`, edition 2024, Gekkotron author, NOT in workspace, `[profile.release] lto = true, opt-level = "z", strip = "symbols"`).
      FR patterns (EN deferred): `"mets un minuteur de {duration}"`, `"minuteur {duration}"`, `"réveille-moi dans {duration}"`. Slot `duration: SlotKind::String`. Guest-side `parse_fr_duration` (in `skills-timer/src/duration.rs`, unit-tested) handles seconds/minutes/hours + `un/une/deux/trois/quatre/cinq/six/sept/huit/neuf/dix`. Reject > 24 h with `SkillResponse::Speak("désolé, je ne gère que les minuteurs de moins de vingt-quatre heures")`.
      `handle`: compute `fires_at_ms = now_ms + parsed_ms`; call `ctx.schedule_mqtt(fires_at_ms, "athena/skills/timer/expired", <compact-JSON {seconds}>)`; `state_set("timer/{returned_id}", <duration_seconds_le_bytes>)`; respond `SkillResponse::Speak("d'accord, minuteur de <duration> lancé")`.
      Runtime wiring for the expired-notification: `wasm/scheduler.rs` emits `Event::SkillNotify { session, skill, text }` directly (rather than round-tripping through an MQTT subscribe on `athena/skills/+/expired`); a small `spawn_skill_notify_forwarder` task subscribes to the event bus and forwards `SkillNotify.text` into the router's TTS token channel, bypassing the intent matcher without threading a new dependency through `RouterDeps`.
      Integration test `crates/athena-voice-runtime/tests/timer_end_to_end.rs`: in-memory `SqliteStore` + `skills-timer.wasm` + fake MQTT client that captures publishes; feed `"mets un minuteur de deux secondes"` as a final transcript; assert `Event::IntentMatched(timer.set)` + `SkillInvoked` + a TTS chunk containing `"d'accord, minuteur"`; sleep past the two-second duration in real wall-clock time (tokio's paused-clock utilities only mock tokio's own timer, not `chrono::Utc::now()`, which both the guest and the scheduler read); assert `Event::ScheduledFired` AND a follow-up TTS chunk with the expiration announce.
      Success criteria: `cargo test --workspace` green including migration test + new store tests + integration test; existing smoke-test integration test still green (regression pin); clippy + fmt clean; `skills-timer.wasm` builds via `cargo build --target wasm32-wasip1 --manifest-path skills-timer/Cargo.toml`.

- [x] Task B — Skill hot-reload (dev mode)
      Add `[skills].hot_reload = false` (default false) to `athena.example.toml` and the config loader. When true, the runtime spawns a filesystem watcher on `[skills].dir`.
      New `wasm/watcher.rs`: uses the `notify` crate via `notify-debouncer-full` (~250 ms debounce) on the skills dir; emits `WatchEvent { path, kind: Added | Modified | Removed }` via internal `mpsc`. Add `notify` + `notify-debouncer-full` to workspace deps.
      Refactor `SkillRegistry` to hold `patterns: Arc<ArcSwap<RuleIndex>>` and `plugins: Arc<RwLock<HashMap<String, Arc<Mutex<dyn SkillPlugin>>>>>`. Add `arc-swap` to workspace deps. `RouterDeps.rules` and `SkillDispatcherHandle`'s lookup path become `Arc<ArcSwap<RuleIndex>>` / RwLock-guarded so a swap under a name is visible to the next dispatch without router restart.
      `SkillRegistry::reload_path(path, deps)`: rebuilds a plugin for a single file and re-runs `install`. On failure, log at `warn` and keep the previous plugin (never a half-loaded state). Emit `Event::SkillReloaded { name }` on success; `Event::SkillReloadFailed { name, reason }` on error. Add both variants to `athena-voice-core/src/event.rs`.
      `SkillRegistry::remove(name)`: drops the plugin and rebuilds the aggregate `RuleIndex` from what remains, then `patterns.store(Arc::new(new_index))`.
      New tokio task `spawn_hot_reload_task(watcher_rx, registry, deps)` in `wasm/mod.rs`. `Runtime::spawn` conditionally starts it when the flag is on.
      Tests (`wasm/registry.rs` + new `wasm/watcher.rs`): (1) `install → remove` clears both the plugin map and the rule index for that name; (2) `install → install` (same name) replaces the plugin and re-populates its rules only; (3) `reload_path` on a broken plugin returns error, emits `SkillReloadFailed`, leaves prior plugin intact; (4) tempdir watcher test — creating, modifying, removing a fixture `.wasm` yields exactly one Added / one Modified / one Removed event within 500 ms.
      Deferred: signature verification on reload (lands with the future signed-WASM task).
      Success criteria: `cargo nextest run --workspace` green including the 4 new tests; `cargo clippy --workspace --all-features --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean; `athena.example.toml` documents `hot_reload`.

- [x] Task A — Barge-in on new final transcript
      Add `Event::BargeIn { session, reason: BargeInReason }` (`NewFinalTranscript`, `VadSpeechStart` reserved) and `Event::SkillCancelled { session, skill }` to `athena-voice-core/src/event.rs`.
      In `pipeline/router.rs`, keep a per-session `utterance_epoch: u64`; bump on every final transcript. Snapshot the epoch before awaiting `dispatcher.call(...)`; if it has moved on when the call resolves, emit `Event::SkillCancelled` and drop the `SkillResponse::Speak/AskLlm` instead of forwarding to TTS/LLM.
      Emit `Event::BargeIn { reason: NewFinalTranscript }` whenever a final transcript arrives while a prior utterance's dispatch or TTS is still in flight (epoch > 0 with pending work).
      `pipeline/tts.rs` (and `sink.rs` if it buffers) subscribe to `Event::BargeIn` and flush queued/streaming speech tokens so the previous response stops playing.
      Add a `CancellationToken` per dispatch so the awaiting side of `SkillDispatcherHandle::call` can bail out immediately; the `spawn_blocking` WASM task finishes naturally and its result is dropped (Extism can't be interrupted mid-call).
      Tests in `pipeline/router.rs`: (1) two rapid final transcripts — first dispatch's speech is dropped, only second reaches `tts_tok_tx`, `Event::SkillCancelled` + `Event::BargeIn` observed; (2) single final transcript — no `BargeIn`/`SkillCancelled` (regression); (3) a mock TTS observing `Event::BargeIn` flushes its buffer.
      Deferred (do NOT scope here): VAD-driven barge-in on `VadSpeechStart`. That requires a real VAD upgrade and lands in a separate task.
      Success criteria: `cargo nextest run --workspace` green including the 3 new tests; `cargo clippy --workspace --all-features --all-targets -- -D warnings` clean; `cargo fmt --all --check` clean.

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
