# Plan

Current state (2026-07-30): the full voice loop works end to end and is
verified live — client --microphone/--wav → whisper.cpp STT worker → intent
matcher → WASM skills (time / weather / timer / home / Jeedom) → TTS over
MQTT (`athena-voice-tts-worker`, macOS `say` engine) → PCM chunks → client
playback via rodio. LLM fallback is opt-in (`llm = "none"` default;
`ollama` and `openai_compatible` providers available). A web admin UI
(`athena-voice-admin`) handles config editing, Jeedom connection test, and
sensor discovery. See `README.md` / `quickstart.sh` for setup and
`athena.voice.toml` for the full-voice config. Remaining in the voice path:
honest tts/meta, Piper TTS engine (last macOS-only piece), wake-word
detection (today the client streams on demand, no hands-free trigger).

## Notes

- The previous revision of this file listed Plans 6–9 as open Backlog while
  their (broken) implementations were already committed, so ticks kept
  re-dispatching finished work. Before adding a task here, check the code —
  and before implementing one, verify its assumptions against the tree.
- DESIGN PRINCIPLE (owner requirement): this project is for ANYONE to
  run, on any OS — not a single-machine setup. Prefer cross-platform
  engines and pure-Rust deps; macOS-only conveniences (`say`,
  `afconvert`) may be defaults but never the only path; minimize setup
  steps and fail fast with actionable messages; keep locale plumbing
  generic (French-first is fine, English must stay reachable).
- Ground rules learned the hard way, for every worker session: never invent
  SDK/provider APIs (read the real ones first); run the affected crate's
  tests plus `./SHOWCASE.sh` before claiming success; wasm skills live
  OUTSIDE the host workspace (see `[workspace] exclude`); MQTT messages
  above ~10 KiB need `set_max_packet_size` on every connection involved.
- Both MQTT providers had latent bugs of the same family (streams never
  terminated, packet caps, route leak, STT discarding audio entirely) —
  all fixed; the wire protocols are documented in `mqtt_tts.rs` /
  `mqtt_stt.rs` module docs. Mirror those patterns in any new provider.

## Backlog

- [ ] Honest audio format metadata in tts/meta (do this BEFORE the Piper task — Piper models have their own native rates, so the metadata must stop lying first)
      `pipeline/sink.rs` hardcodes `{codec: "opus", sample_rate: 24000}` in the session `tts/meta` message while the actual stream today is s16le at the worker's rate.
      Thread real format info: extend the TTS provider trait (or wrap AudioStream) so synthesize returns format metadata alongside chunks; FakeTts reports a `text` pseudo-codec, MqttTts forwards what the worker declares (add optional `format`/`sample_rate` fields to the worker's first response message; missing fields default to s16le/22050 for compatibility).
      Update the satellite client to honor tts/meta instead of the --rate flag when metadata is present (keep --rate as override).
      Success criteria: (a) client plays correctly with no --rate flag against both fake and say-worker configs; (b) meta reflects reality for each provider; (c) runtime + provider tests green.

- [ ] Piper engine option for the TTS worker (PORTABILITY: `say` is the only macOS-only piece left in the voice path)
      Add a `--engine piper --piper-bin <path> --piper-model <path>` mode to `crates/athena-voice-tts-worker` alongside the default `say` engine, replacing only `synthesize_wav` (the wire protocol must not change).
      Piper CLI outputs WAV at the model's native rate; declare that real rate in the worker's response metadata (the tts/meta task above threads it through), so no resampling is needed.
      Do NOT vendor models; document where to fetch a French voice (e.g. fr_FR-siwis-medium) and add the paths to the config header.
      Success criteria: (a) with a downloaded Piper voice the full loop speaks with the Piper voice on Linux and macOS; (b) `say` remains the default with unchanged behavior; (c) startup fails fast with a clear message when the binary/model paths are wrong.

- [ ] Wake-word detection in the satellite client (hands-free trigger — today --microphone streams on demand only)
      Evaluate pure-Rust cross-platform detectors first (e.g. rustpotter) — read the real crate API before writing code, do not invent it; a subprocess engine is acceptable fallback but pure-Rust is strongly preferred per the portability principle.
      Add an opt-in client mode (e.g. --wake-word <model/config path>) that listens continuously, opens a session and starts streaming only after detection, and re-arms after the session ends; keep detection entirely client-side (no wake audio leaves the satellite).
      Without the flag, current behavior must be byte-for-byte unchanged; feature-gate heavy deps if needed.
      Do NOT vendor wake models; document where to fetch or how to train one in the config header.
      Success criteria: (a) live-verified hands-free round trip: spoken wake word → question → spoken answer, without touching the keyboard; (b) no session traffic before detection; (c) existing client modes and tests unchanged and green.

## In progress

## Done

- [x] Docker Compose delivery with prebuilt GHCR image (2026-08-02)
      Multi-stage Dockerfile (rust:1.95 build → bookworm-slim runtime, non-root, /data volume) bundling the smoke-test/weather/jeedom skills and athena.docker.toml (broker via ATHENA__MQTT__* env). docker-compose.yml with optional broker-profile mosquitto, .env.example, update.sh (pull --rebase + compose pull + up -d). CI `docker` job publishes ghcr.io/gekkotron/athena-voice (latest + sha) after nextest passes. Image built and published by CI (run 30762052625, first attempt); runtime round-trip to be verified on the GEEKOM after the owner makes the GHCR package public. MANUAL owner step pending: make the GHCR package public. Spec: docs/superpowers/specs/2026-08-02-docker-compose-delivery-design.md.

- [x] Assist text bridge + GEEKOM Linux profile (2026-07-31)
      New `[assist]` block: runtime subscribes `assist/transcription/+` on the LAN broker, routes text through the normal intent/skill/LLM pipeline per device, and answers as `{"text": …}` on `assist/tts/{device}` with loader statuses on `assist/llm/{device}/status` — DomoticApp protocol, zero app changes. SentenceBuffer extracted from the TTS actor and shared. `athena.assist.toml` profile (no audio providers used), README Linux run-book + systemd unit, ci.yml YAML repaired and Linux toolchain/deps fixed (fmt+coverage green; clippy/deny red on pre-existing debt). Live-verified with mosquitto_pub/sub. Spec: docs/superpowers/specs/2026-07-31-assist-bridge-geekom-design.md.

- [x] Redact the Jeedom API key from HTTP error logs (2026-07-30)
      Host boundary fix in `wasm/host_fns.rs`: new `redact_query_values` scrubs every query-param value (→ `REDACTED`) from error text while keeping param names, scheme, host, and path; the fetch path is extracted into a testable `fetch_json` that redacts both send and JSON-decode errors before they become skill-visible `{"error": …}` payloads. Regression test proves it live: reqwest 0.12 really does embed `?apikey=SUPERSECRET` in its connect-error text (watched the test fail first), and the redacted error still names the host. 255 workspace tests + SHOWCASE.sh green.

- [x] Web admin UI + Jeedom skill (2026-07-25..29)
      `athena-voice-admin` crate: web config editor with validation, secrets protection, upload quarantine, Jeedom connection test, and streaming sensor-discovery endpoint with size cap; admin UI discovery tree. `skills-jeedom` WASM skill: room queries, device enumeration, spoken binary states, names composed from equipment for generic commands. All on origin/main.

- [x] LLM made truly optional + openai_compatible provider (2026-07-24)
      Owner preference: no OpenAI/cloud dependency. `llm = "none"` is now the shipped default — unmatched questions get a deterministic spoken capabilities answer (FR/EN, unit-tested, verified live). For those who opt in: new `openai_compatible` provider (SSE chat/completions; works with hosted APIs, llama.cpp, vLLM, Ollama /v1; bearer token only via api_key_env env var, fail-fast when missing; mockito tests for streaming/auth header/malformed/termination/refused). Ollama stays as the second opt-in. NOT yet live-verified against a real /v1 endpoint end to end — protocol is pinned by tests; config examples in athena.voice.toml.

- [x] Idle-session reaper (2026-07-24)
      SessionManager tracks last inbound activity (audio/text touch); a runtime task ticks every 10 s and closes sessions idle past `[server] session_idle_secs` (default 120) through the normal close path, so `done`/`SessionEnded` fire. Unit-tested (idle reaped, touched survives) and verified live with a kill -9'd client at a 15 s override.

- [x] One-command quickstart + real README (2026-07-24)
      ./quickstart.sh: detects broker (running / mosquitto / docker fallback), offers the whisper model download, builds whisper.cpp + skills + binaries, picks the richest mode (voice/say/fake), starts everything with prefixed logs and trap-based teardown (process substitution so $! is the worker, not the log prefixer), readiness-probes with a real client call. shellcheck-clean; verified live including SIGTERM teardown. README rewritten: architecture, modes table, satellite protocol table, manual setup, dev workflow.

- [x] English pattern coverage for the bundled skills (2026-07-24)
      sdk Intent carries the session locale (serde default keeps wire compat; router injects it at dispatch). smoke-test time, weather (patterns, responses, WMO phrases, geocoding language param), and timer (parse_en_duration) answer in English for locale "en"; configs ship locales = ["fr", "en"]. Verified live: EN voice ("What time is it" → "it is 3:14 PM"), EN weather with real data, FR regression intact; new en_end_to_end integration test.

- [x] Ollama LLM fallback verified live + token-contract fix (2026-07-24)
      llama3.2:1b via `brew install ollama` answers unmatched/compound questions in French, streamed sentence-by-sentence through say TTS. Fixed en route: TTS token channel is now VERBATIM fragments (LLMs stream sub-word pieces with their own spacing — the old space-joining would have garbled real LLM text); LLM actor speaks a locale-aware apology when the backend fails; ollama.rs edge-case tests (refused/malformed/no-done). Weather skill gained temperature phrasings ("quelle est la température (extérieure)"). NB: Ollama is too slow on the GEEKOM target — see the remote-LLM backlog task.

- [x] Voice input end to end: mqtt_stt fix, whisper worker, client audio modes (2026-07-24)
      mqtt_stt now actually delivers audio (base64 frames + utterance-boundary markers) and its transcript stream terminates; new athena-voice-stt-worker (whisper.cpp engine, models under ./models are placeholders — real ones are gitignored downloads); client --wav/--microphone capture with resampling to the s16le/16k contract; empty audio frame = end-of-utterance marker through ingest; TTS actor idle-flush so unpunctuated LLM answers are spoken; athena.voice.toml full-voice config. Verified live: spoken WAV in → whisper transcript → weather skill → say synthesis → client playback.

- [x] Repair generated-code corruption across SDK, storage, runtime, server (2026-07-23/24)
      Rewrote skill SDK host bindings; storage retention on timestamp_sec column; restored Runtime::spawn; fixed manifests, vendored webrtcvad stub, honest server tests; workspace check and 184 workspace tests green. Commit "Repair generated-code corruption; wire the full skill voice loop".

- [x] Satellite text-injection topic + skill loading in serve (2026-07-24)
      New `athena/sat/<sat>/session/<sid>/text` ingress bypassing STT; `Runtime::spawn` loads skills + dispatcher from `[skills]` config; per-pid MQTT client ids.

- [x] MQTT satellite client (2026-07-24)
      `athena-voice-client` rewritten: --text injection, session lifecycle, --speak (macOS say), --play (rodio PCM playback).

- [x] Real TTS over MQTT (2026-07-24)
      `athena-voice-tts-worker` (say engine) speaking the mqtt_tts protocol; fixed provider stream termination, packet caps, route leak; `athena.say.toml`; verified live (spoken time and weather answers).

- [x] host_local_time host function + real time skill (2026-07-24)
      Host serves epoch + UTC offset; SDK exposes LocalTime; smoke-test skill speaks actual local time.

- [x] Cross-platform audio sink (2026-07-24)
      rodio-based AudioSink behind feature `audio` (replaces Linux-only pipewire); AudioChunk carries sample_rate; VolumeChanged event applied by the sink.

## Manual checklist (human, not the orchestrator)

- [ ] Rotate the Jeedom API key that was used during web-UI development, then browser-check the admin UI end to end.
- [ ] Decide the fate of `athena-voice-server`: its socket-based VAD→hotword→ASR path duplicates the runtime's MQTT satellite pipeline and is mostly stubs (audio socket unimplemented, Piper tokenizer TODO, placeholder models). The wake-word backlog task covers the one capability it was meant to add — once that lands, deleting the crate is the likely call.
- [ ] Download a real whisper ggml model and a Piper French voice into `models/` (the current files are byte-sized placeholders).
- [ ] Consider enabling `[skills] hot_reload = true` in dev configs now that the watcher is deflaked.
