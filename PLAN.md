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
