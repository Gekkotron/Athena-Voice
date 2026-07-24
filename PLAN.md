# Plan

Current state (2026-07-24): the text-driven voice loop works end to end and
is verified live — MQTT satellite client → text-injection topic → intent
matcher → WASM skills (time / weather / timer / home) → TTS over MQTT
(`athena-voice-tts-worker`, macOS `say` engine) → PCM chunks → client
playback via rodio. `cargo check --workspace --all-targets` is clean and
`cargo nextest run --workspace` is green (189 tests). See `SHOWCASE.md` for
the quick-start and `athena.local.toml` / `athena.say.toml` for runnable
configs. Voice input works too (whisper.cpp worker + client --wav/--microphone; see athena.voice.toml) — verified with a spoken WAV end to end. Remaining: Ollama LLM fallback, Piper TTS engine, honest tts/meta.

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

- [ ] One-command quickstart for new users
      Today a new user needs mosquitto + two workers + serve + client, plus whisper.cpp built and a model downloaded — five terminals and tribal knowledge. Add a `quickstart.sh` at the repo root that: checks prerequisites (cargo, mosquitto or offers `docker run eclipse-mosquitto`, cmake); builds whisper-cli if missing; downloads ggml-small.bin if missing (with size warning); builds all skills and binaries; then starts broker-check + stt worker + tts worker + serve with athena.voice.toml under one process group with clean Ctrl-C teardown and prefixed log lines.
      Also add a matching README section (the current README is 466 bytes) covering Linux (apt paths, Piper instead of say once available) and macOS.
      Success criteria: (a) fresh clone → `./quickstart.sh` → `cargo run -p athena-voice-client -- --text "météo à Paris"` answers, with no other manual steps besides model download consent; (b) Ctrl-C stops everything it started; (c) script is POSIX-friendly bash checked with shellcheck.

- [ ] Remote LLM provider for constrained targets (GEEKOM)
      Local Ollama inference is too slow on the GEEKOM deployment target. Two paths, both config-only for the runtime: (1) point `base_url` at an Ollama served from a beefier LAN host (works today — the provider is just HTTP); (2) add an `openai_compatible` StageChoice variant in `crates/athena-voice-providers` (chat/completions streaming, api key from env var, base_url + model in config) so hosted APIs and llama.cpp/vLLM servers all work through one provider.
      Mirror the ollama.rs patterns: mockito tests for streaming, HTTP errors, connection refused, malformed chunks, termination; the pipeline's spoken-apology fallback already covers backend death.
      Never commit API keys; read them from an env var named in the config.
      Success criteria: (a) unmatched questions get spoken LLM answers with a remote endpoint configured; (b) provider tests green without network; (c) config documented in athena.voice.toml comments.

- [ ] English pattern coverage for the bundled skills
      The locale plumbing is generic ([skills] pattern_rules(locale) per locale, Locale on sessions) but every bundled skill returns patterns only for "fr". Add EN phrases and responses to smoke-test (time), weather, and timer (duration parser needs an EN counterpart to parse_fr_duration), keyed off the locale argument.
      Keep FR behavior byte-identical (regression: existing integration tests must not change).
      Success criteria: (a) a session started with locale "en" matches "what time is it" / "weather in {city}" / "set a timer for {duration}" and answers in English; (b) new integration test drives one EN utterance end to end; (c) FR tests untouched and green.

- [ ] Piper engine option for the TTS worker (PORTABILITY: `say` is the only macOS-only piece left in the voice path)
      Add a `--engine piper --piper-bin <path> --piper-model <path>` mode to `crates/athena-voice-tts-worker` alongside the default `say` engine, replacing only `synthesize_wav` (the wire protocol must not change).
      Piper CLI outputs WAV at the model's native rate; resample or pass the actual rate in the response if it differs from --rate (keep it simple: require the worker's --rate to match the model and validate at startup).
      Do NOT vendor models; document where to fetch a French voice (e.g. fr_FR-siwis-medium) and add the paths to the config header.
      Success criteria: (a) with a downloaded Piper voice the full loop speaks with the Piper voice on Linux and macOS; (b) `say` remains the default with unchanged behavior; (c) startup fails fast with a clear message when the binary/model paths are wrong.

- [ ] Honest audio format metadata in tts/meta
      `pipeline/sink.rs` hardcodes `{codec: "opus", sample_rate: 24000}` in the session `tts/meta` message while the actual stream today is s16le at the worker's rate.
      Thread real format info: extend the TTS provider trait (or wrap AudioStream) so synthesize returns format metadata alongside chunks; FakeTts reports a `text` pseudo-codec, MqttTts forwards what the worker declares (add optional `format`/`sample_rate` fields to the worker's first response message; missing fields default to s16le/22050 for compatibility).
      Update the satellite client to honor tts/meta instead of the --rate flag when metadata is present (keep --rate as override).
      Success criteria: (a) client plays correctly with no --rate flag against both fake and say-worker configs; (b) meta reflects reality for each provider; (c) runtime + provider tests green.

## In progress

## Done

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

- [ ] Decide the fate of `athena-voice-server`: its socket-based VAD→hotword→ASR path duplicates the runtime's MQTT satellite pipeline and is mostly stubs (audio socket unimplemented, Piper tokenizer TODO, placeholder models). Consolidate on the MQTT path, or invest in the socket path — don't let both drift.
- [ ] Download a real whisper ggml model and a Piper French voice into `models/` (the current files are byte-sized placeholders).
- [ ] Consider enabling `[skills] hot_reload = true` in dev configs now that the watcher is deflaked.
