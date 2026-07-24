# Plan

Current state (2026-07-24): the text-driven voice loop works end to end and
is verified live — MQTT satellite client → text-injection topic → intent
matcher → WASM skills (time / weather / timer / home) → TTS over MQTT
(`athena-voice-tts-worker`, macOS `say` engine) → PCM chunks → client
playback via rodio. `cargo check --workspace --all-targets` is clean and
`cargo nextest run --workspace` is green (184 tests). See `SHOWCASE.md` for
the quick-start and `athena.local.toml` / `athena.say.toml` for runnable
configs. The missing half is voice INPUT: audio capture and real STT.

## Notes

- The previous revision of this file listed Plans 6–9 as open Backlog while
  their (broken) implementations were already committed, so ticks kept
  re-dispatching finished work. Before adding a task here, check the code —
  and before implementing one, verify its assumptions against the tree.
- Ground rules learned the hard way, for every worker session: never invent
  SDK/provider APIs (read the real ones first); run the affected crate's
  tests plus `./SHOWCASE.sh` before claiming success; wasm skills live
  OUTSIDE the host workspace (see `[workspace] exclude`); MQTT messages
  above ~10 KiB need `set_max_packet_size` on every connection involved.
- The `mqtt_tts` provider had three latent bugs (stream never terminated,
  packet caps, route leak) — fixed in `mqtt_tts.rs`/`mqtt_client.rs`. The
  STT twin was NOT audited; that is the first Backlog task.

## Backlog

- [ ] Audit the mqtt_stt provider against the fixed mqtt_tts patterns
      Read `crates/athena-voice-providers/src/remote/mqtt_stt.rs` and compare with the fixes applied to `mqtt_tts.rs` and `mqtt_client.rs` (commit "Real TTS over MQTT").
      Check: response handling terminates on the protocol's `done`/final marker and on timeout (never a hanging stream/future); large audio payloads fit the packet caps (requests carry base64 or raw audio — compute worst case for 10 s of s16le@16kHz); session routes are cleaned up; the wire format doc comment matches what the code actually sends.
      Fix what deviates, mirroring the TTS-side patterns; extend the doc comment with the exact request/response JSON schema a worker must implement.
      Success criteria: (a) `cargo nextest run -p athena-voice-providers` green; (b) the wire protocol is documented precisely enough to write a worker without reading the provider source; (c) no code path can block a session forever if the worker dies mid-request.

- [ ] Whisper STT worker crate (athena-voice-stt-worker)
      New workspace member `crates/athena-voice-stt-worker`, modeled on `crates/athena-voice-tts-worker` (same CLI shape: --host/--port/--name, pid-suffixed client id, `set_max_packet_size`).
      Subscribe to `athena/providers/stt/<name>/request` and answer on `.../response` per the wire format documented by the audit task above.
      Engine: shell out to the whisper.cpp CLI (`whisper-cli`/`main` binary; the submodule is at `./whisper.cpp`, models under `./models/` — note `ggml-small-french-q5_1.bin` there is a 29-byte placeholder, so the worker must take a `--model <path>` flag and fail with a clear message when the file is not a real model). Convert incoming PCM to a temp WAV with `hound`, run whisper with `-l fr`, return the transcript text.
      Add `athena.voice.toml` (or extend `athena.say.toml`) with `stt = { mqtt_stt = { name = "whisper" } }` and a header documenting the 4-process setup (broker, stt worker, tts worker, serve).
      Success criteria: (a) worker + serve + client run locally; (b) feeding a WAV with French speech through the pipeline produces a transcript event and a skill answer; (c) worker absence degrades to a timeout, not a hang; (d) unit test for the PCM→WAV conversion.

- [ ] Client --microphone mode (voice input from the Mac)
      Extend `crates/athena-voice-client` with a `--microphone` mode: capture from the default input device via `cpal`, downmix to mono s16le at 16 kHz (or the rate the STT worker expects — align the two flags), and publish frames to `athena/sat/<sat>/session/<sid>/audio` in ~100 ms chunks.
      Session flow: start session, stream audio while a key is held or until `--duration-secs` elapses, publish `end`, then print/play the response like the --text mode does.
      Keep --text mode untouched; --microphone and --text are mutually exclusive arguments.
      The runtime's ingest→vad→stt actors already exist; verify the vad actor's frame expectations (25 ms at what rate — read `pipeline/vad.rs` and `pipeline/ingest.rs` first) and resample client-side to match.
      Success criteria: (a) speaking "quelle heure est-il" into the mic yields a transcript event from the STT worker and a spoken answer with --play; (b) works together with the whisper worker task above; (c) graceful error when no input device exists.

- [ ] Verify the Ollama LLM fallback end to end
      `crates/athena-voice-providers/src/remote/ollama.rs` exists but has never been exercised. Configure `llm = { ollama = { base_url = "http://localhost:11434", model = "<small local model>" } }` in a config variant and drive an unmatched utterance ("raconte-moi une blague") through the client.
      Audit ollama.rs the same way as the MQTT providers: token stream termination, timeouts, error surfaces (connection refused must yield a spoken apology or clean LlmFallback failure, not a hang).
      Document in the config header that Ollama must be installed and which model was tested.
      Success criteria: (a) unmatched intents produce an LLM-generated spoken answer when Ollama runs; (b) with Ollama down, the session still completes with a clean failure path; (c) any bugs found are fixed with tests.

- [ ] Piper engine option for the TTS worker
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
