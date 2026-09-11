# Plan

Current state (2026-07-30): the full voice loop works end to end and is
verified live — client --microphone/--wav → whisper.cpp STT worker → intent
matcher → WASM skills (time / weather / timer / home / Jeedom) → TTS over
MQTT (`athena-voice-tts-worker`, macOS `say` engine) → PCM chunks → client
playback via rodio. LLM fallback is opt-in (`llm = "none"` default;
`ollama` and `openai_compatible` providers available). A web admin UI
(`athena-voice-admin`) handles config editing, Jeedom connection test, and
sensor discovery. See `README.md` / `quickstart.sh` for setup and
`athena.voice.toml` for the full-voice config.

Since 2026-09-11 the voice path is portable and honest: Piper joins `say`
as a TTS engine, `tts/meta` declares each provider's real format, and
`docker compose --profile voice` ships the whole stack for a Linux box.
An ESP32 satellite lives in `satellite-esp32/` (push-to-talk on classic
ESP32 and S3, plus an on-device "Alexa" wake word on the S3). Remaining
in the voice path: wake-word detection in the *CLI* client (today it
streams on demand, no hands-free trigger).

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

- [ ] Wake-word detection in the satellite client (hands-free trigger — today --microphone streams on demand only)
      Evaluate pure-Rust cross-platform detectors first (e.g. rustpotter) — read the real crate API before writing code, do not invent it; a subprocess engine is acceptable fallback but pure-Rust is strongly preferred per the portability principle.
      Add an opt-in client mode (e.g. --wake-word <model/config path>) that listens continuously, opens a session and starts streaming only after detection, and re-arms after the session ends; keep detection entirely client-side (no wake audio leaves the satellite).
      Without the flag, current behavior must be byte-for-byte unchanged; feature-gate heavy deps if needed.
      Do NOT vendor wake models; document where to fetch or how to train one in the config header.
      Success criteria: (a) live-verified hands-free round trip: spoken wake word → question → spoken answer, without touching the keyboard; (b) no session traffic before detection; (c) existing client modes and tests unchanged and green.

## In progress

## Done

- [x] Voice-enabled Docker delivery for audio satellites (2026-09-11)
      The container profile shipped `fake` STT/TTS (right for the text-assist bridge, useless for the ESP32 satellite). The image now builds the stt/tts workers plus whisper-cli (whisper.cpp cloned at the submodule's pinned commit — the submodule is .dockerignore'd and CI checkouts don't fetch it) and installs Piper in a venv; docker-compose gained a `voice` profile (stt-worker + tts-worker, no /data, no ports, ./models bind-mounted read-only with filenames overridable via .env); athena.docker.voice.example.toml carries the real mqtt_stt/mqtt_tts providers with text assist still enabled alongside. README documents assist-vs-voice shapes and the model downloads (ggml-base as the mini-PC default). The Piper worker synthesizes one word at startup before announcing readiness — `--help` proves nothing about the native stack, and this caught a broken macOS piper wheel (espeak-ng data compiled to an absent CI path) that would have hung every session. NOT VERIFIED LOCALLY: the image build (this Mac has no Docker daemon); CI's docker job is the first real build, and the GEEKOM pull should follow a green run. Live Piper synthesis on Linux still unproven — the macOS wheel is broken upstream, so only the stub-driven code path was exercised end-to-end.

- [x] Piper engine option for the TTS worker (2026-09-11)
      `--engine say|piper` in `athena-voice-tts-worker`; piper runs as `--model <onnx> --output_file <wav>` with text on stdin — the form both piper1-gpl and the legacy binary accept, verified against piper's own argument parser rather than guessed. Synthesis returns the rate alongside the samples (Piper voices have native rates), so the declared metadata carries the real value and nothing resamples; multi-channel output is downmixed rather than shipped as interleaved "mono". Bad --piper-bin/--piper-model fail at startup with actionable messages. Models are not vendored; README documents the fr_FR-siwis-medium download. Tests use a stub piper script, proving the WAV header's rate is reported rather than a constant, without installing Piper. `say` remains the default and was verified unchanged live.

- [x] Honest audio format metadata in tts/meta (2026-09-11)
      `pipeline/sink.rs` hardcoded `{codec: "opus", sample_rate: 24000}` while every real provider streamed s16le at its own rate. Now `Tts::synthesize` returns `TtsAudio { format, sample_rate, stream }`; the MQTT worker protocol declares `{format, sample_rate, channels}` in its first response message (absent = s16le/22050, so pre-metadata workers keep working); FakeTts reports a `text` pseudo-codec so text chunks are no longer advertised as audio; the TTS stage forwards the format to the sink (SinkMsg::Format), which publishes it once and warns on a mid-session format change; the satellite client honours tts/meta for playback with --rate demoted to an override. Verified live for both engines: say declares s16le/22050, a 16 kHz Piper model declares 16000.

- [x] satellite-esp32: on-device "Alexa" wake word via esp-sr WakeNet, ESP32-S3 only (2026-09-11)
      Owner switched the wake word from Jarvis to Alexa. esp-sr 2.5.3 added via esp-idf-sys extra_components (bindings module `sr`); src/wake.rs wraps WakeNet (esp_srmodel_init on the `model` partition → esp_wn_handle_from_name → detect with internal chunk buffering); idle-state mic frames feed it on the S3 and a detection acts like a BOOT press, disarmed while a session is active. Custom partitions_s3.csv (staged into embuild's CMake project via ESP_IDF_GLOB_PARTTABLE_* env) + sdkconfig.defaults.esp32s3 select wn9_alexa; flash-s3.sh flashes app + partition table + srmodels.bin (espflash alone skips the model). Verified: both targets compile clean, 22 host tests green, srmodels.bin (284K) produced. CONFIRMED LIMITATION: esp-sr's Kconfig gates all WakeNet9/10 models to S3/P4 and ships no usable classic-ESP32 models, so the owner's WROOM keeps push-to-talk — hands-free on the WROOM would need different hardware (S3) per the "no wake audio leaves the satellite" rule. Live "Alexa" round trip pending S3 hardware.

- [x] Assist heartbeat for the app's online indicator (2026-08-03)
      The bridge publishes `{"timestamp": epoch-secs, "millis": uptime-ms, "uptime_minutes": …}` to `assist/heartbeat/` (trailing slash = the app's literal subscription) every 10 s; the DomoticApp flags the assistant offline after 16 s without a beat and reads only `timestamp`. Unit-tested; spawned from Runtime wiring so bridge tests stay deterministic.

- [x] Admin UI: token authentication removed (2026-08-03)
      Owner decision for the home-LAN deployment: the web UI is now open — no token, no first-run print. Secrets stay write-only (GET masks values, blank writes never clobber stored secrets — tests unchanged). README documents the trade-off and the 127.0.0.1 bind for restricting access.

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
