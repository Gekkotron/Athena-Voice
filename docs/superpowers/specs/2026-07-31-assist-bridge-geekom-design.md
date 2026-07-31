# Assist bridge + GEEKOM deployment — design

Date: 2026-07-31
Status: approved by owner (sections reviewed 2026-07-31)

## Goal

A first version of Athena-Voice running on the owner's GEEKOM mini-PC
(Linux, installed by cloning this repo), answering voice queries sent from
the owner's Android home-automation app (DomoticApp). The app already does
speech-to-text and text-to-speech **on the phone** and talks to the LAN
MQTT broker through an encrypted HTTP/SSE gateway, so v1 is a **text
bridge**: no audio bytes cross the network and no audio engine runs on the
GEEKOM.

## Context (verified against the app's source)

DomoticApp (`com.geckostudio.domoticapp`, Kotlin/Android) assistant flow:

- Publishes questions as JSON `{"text": "..."}` to MQTT topic
  `assist/transcription/{device_id}` (on-device STT, wake word supported).
- Subscribes (via the gateway's SSE endpoint) to `assist/tts/{device_id}`;
  each message's `{"text": "..."}` is spoken with Android `TextToSpeech`
  (`QUEUE_ADD`, so consecutive messages speak in order) and shown as a chat
  bubble.
- Subscribes to `assist/llm/{device_id}/status`; `{"status": "in progress"}`
  shows a loader bubble, any other status clears it.
- A previous backend ("Assist", port 9000) consumed these topics; it will be
  shut down. Athena claims the same topics — **the app needs zero changes**.

## Architecture

New module `crates/athena-voice-runtime/src/assist/`, enabled by an
optional `[assist]` config block. One process on the GEEKOM
(`athena-voice-cli serve`): runtime + WASM skills + assist bridge,
connected to the existing LAN broker. No STT/TTS providers configured.

### Ingress

- Subscribe to `{prefix}/transcription/+` (prefix default `assist`).
- Parse the device id from the last topic segment. Reject (log `warn`,
  drop) ids that are empty or contain `/`, `+`, or `#` so a hostile
  publisher cannot steer Athena's answer topic.
- Parse the payload as JSON; require a non-empty `text` field. Malformed or
  empty → log `warn` with topic, drop, publish nothing.

### Sessions

- First message from a device creates a long-lived session with satellite
  id `assist:{device}`, wired like a voice session — same `IntentMatcher`,
  skill dispatcher, LLM fallback, and barge-in (a new question cancels an
  in-flight answer) — but **the pipeline stops at the token channel**: the
  bridge consumes the `mpsc::Receiver<String>` the router/LLM feed instead
  of spawning the TTS actor.
- Session locale comes from `[assist] locale` (default `fr`).
- The existing idle reaper closes idle assist sessions; the next message
  from that device recreates one.

### Answer path

- Aggregate tokens with the same sentence-split + idle-flush semantics as
  the TTS actor (LLM answers start speaking early; skill `Speak` responses
  are typically one message).
- Publish each completed sentence as `{"text": "..."}` to
  `{prefix}/tts/{device}`.
- Publish `{"status": "in progress"}` to `{prefix}/llm/{device}/status`
  when a question is accepted, and `{"status": "done"}` after the answer
  completes — **always**, including on skill/LLM failure or empty answers,
  so the app's loader cannot get stuck. The router's existing locale-aware
  apology on LLM failure flows through as a normal answer.

## Configuration

```toml
[assist]
enabled = true
topic_prefix = "assist"   # {prefix}/transcription/+ in, {prefix}/tts/{device} out
locale = "fr"
```

- Absent `[assist]` block → bridge off, existing behavior untouched.
- `[mqtt]` gains optional `username` and `password_env` (password read from
  the named environment variable; set-but-missing env var is a startup
  error naming the variable — same pattern as the LLM `api_key_env`).
  Neither set → anonymous connection, unchanged.
- A config with `[assist]` enabled and **no** `[providers] stt/tts` must be
  valid: the config loader learns that an assist-only runtime is legal.
- New runnable profile `athena.assist.toml` in the repo root: placeholder
  LAN broker address, bundled skills (time / weather / timer / home /
  Jeedom), `llm = "none"`, `[assist]` enabled, no audio providers.

## Deployment (GEEKOM, Linux, owner-installed)

- README gains a "Run on a Linux box" section: install Rust + C toolchain,
  clone, build skills, edit `athena.assist.toml` (broker address,
  credentials env var if any), `cargo run --release -p athena-voice-cli --
  serve --config athena.assist.toml`; example systemd unit for boot
  persistence. whisper.cpp and audio libraries are NOT needed for this
  profile.
- New GitHub Actions job on `ubuntu-latest`: `cargo check --workspace
  --all-targets` + `cargo nextest run --workspace` (nextest installed via
  `taiki-e/install-action`, which fetches a prebuilt binary), so Linux
  builds are continuously proven before pulling on the GEEKOM.

## Error handling summary

| Failure | Behavior |
| --- | --- |
| Malformed/empty payload | warn + drop, nothing published |
| Hostile/invalid device id | warn + drop |
| Broker connection lost | rumqttc reconnects; bridge re-subscribes; QoS 0 messages sent meanwhile are lost (acceptable v1) |
| Skill/LLM failure | locale-aware apology as answer; `done` status always sent |
| `[assist]` on, broker unreachable at startup | fail fast, actionable message |
| `password_env` set but env var missing | fail fast, names the variable |

## Testing

- **Unit**: topic/device-id parsing (incl. hostile ids), payload parsing,
  sentence aggregation, status ordering (in progress → text → done).
- **Integration** (existing `*_end_to_end` style, in-process runtime +
  broker): question on `assist/transcription/testdev` → time answer on
  `assist/tts/testdev` bracketed by statuses; barge-in (second question
  cancels the first); malformed payload (no crash, nothing published).
- **Live**: mosquitto + serve on the dev Mac, `mosquitto_pub`/`_sub` round
  trip; then the real GEEKOM + DomoticApp.

## Out of scope (deliberate)

- Raw audio from the app to the GEEKOM (whisper on GEEKOM, Piper TTS) —
  future; the topic protocol is unchanged by this design, so adding audio
  later is additive.
- LLM on the GEEKOM (`llm = "none"` in the profile; owner can point
  `openai_compatible` at a remote endpoint later).
- The app's `assist/heartbeat/` topic — ignored in v1.
