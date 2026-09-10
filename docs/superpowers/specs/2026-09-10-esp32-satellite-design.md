# ESP32-S3 voice satellite — design

Date: 2026-09-10
Status: approved

## Goal

A `satellite-esp32/` firmware folder at the repo root: an ESP32-S3 with an
INMP441 I2S microphone and a MAX98357A I2S amplifier that acts as a full
audio satellite for the Athena-Voice runtime over the existing MQTT
satellite protocol (`athena/sat/<sat-id>/session/<uuid>/…`). Wake word
detection runs on-device (phase 2); phase 1 is push-to-talk.

## Non-goals (v1)

- Custom "Athena" wake word — WakeNet custom models require Espressif's
  training service; v1 ships the stock **"Jarvis"** model (user's choice).
- Opus decoding — the runtime's `tts/meta` advertises `codec: "opus"` but
  the bundled TTS worker emits raw s16le PCM and the runtime's own local
  playback drops opus ("not wired yet"). v1 plays raw s16le at the
  advertised `sample_rate`. The meta/codec inconsistency is reported
  upstream as a PLAN.md note, not silently worked around.
- Acoustic echo cancellation, multi-mic beamforming, on-device display.
- OTA updates (flash over USB via `espflash`).

## Hardware

- ESP32-S3 devkit (≥ 8 MB flash; PSRAM helpful but not required).
- INMP441 I2S MEMS microphone (in) — defaults: BCLK GPIO4, WS GPIO5,
  SD GPIO6, L/R tied low (left channel).
- MAX98357A I2S amplifier (out) — defaults: BCLK GPIO15, LRC GPIO16,
  DIN GPIO7.
- BOOT button (GPIO0) = push-to-talk trigger in phase 1.

Pins are constants in `config.rs`; the README documents the wiring and how
to change them.

## Toolchain

- Rust via `espup` (channel `esp`, Xtensa), target `xtensa-esp32s3-espidf`.
- `esp-idf-svc` (std) on ESP-IDF v5.x via `embuild`; flashing/monitoring
  with `espflash` as the cargo runner.
- The folder is excluded from the root workspace (same reasoning and
  comment style as `skills-*`): it cannot share the native host target.
  `cargo check --workspace` at the root remains unaffected.

## Folder layout

```
satellite-esp32/
  Cargo.toml            # standalone package, authors = Gekkotron
  rust-toolchain.toml   # channel = "esp"
  .cargo/config.toml    # target xtensa-esp32s3-espidf, espflash runner
  sdkconfig.defaults
  cfg.example.toml      # Wi-Fi SSID/pass, broker URL, sat-id (toml-cfg)
  build.sh              # mirrors skills-*/build.sh convention
  README.md             # wiring, espup setup, flash & monitor commands
  src/
    main.rs             # boot → Wi-Fi → MQTT → trigger loop
    config.rs           # toml-cfg struct + pin constants
    net.rs              # Wi-Fi connect + EspMqttClient with reconnect/backoff
    mic.rs              # I2S std driver in, 16 kHz mono s16le frames
    speaker.rs          # I2S out queue, sample-rate from tts/meta
    session.rs          # protocol state machine — hardware-free, host-testable
    wake.rs             # phase 2: esp-sr WakeNet FFI
```

Secrets live in a gitignored `cfg.toml` (the `toml-cfg` pattern);
`cfg.example.toml` is committed.

## Protocol & data flow

Session flow against the existing runtime (no server-side changes):

1. Trigger (button, later wake word) → generate a UUID session id, publish
   `{"locale":"fr"}` to `athena/sat/<id>/session/<sid>/start`.
2. Stream mic frames (raw s16le mono 16 kHz, ~20 ms per publish, QoS 0)
   to `…/audio`.
3. End of utterance: RMS energy below threshold for 800 ms, or 10 s max —
   then publish an **empty** `…/audio` payload.
4. Subscribe (before `start`) to `…/transcript`, `…/tts/meta`, `…/tts`,
   `…/tts/text`, `…/done`. Parse `tts/meta` for `sample_rate`; enqueue
   `tts` chunks to the speaker as s16le.
5. `…/done` (or a 30 s no-reply timeout) closes the session and returns
   to idle.

## Session state machine (`session.rs`)

States: `Idle → Streaming → AwaitingReply → Playing → Idle`, with timeout
transitions from every non-idle state back to `Idle`. The module holds no
hardware or MQTT handles — it consumes events (`Trigger`, `MicFrame`,
`SilenceDetected`, `TtsMeta`, `TtsChunk`, `Done`, `Tick`) and emits
commands (`Publish(topic, payload)`, `PlayAudio`, `EndSession`), so it
unit-tests on the host under a default feature that disables the esp-idf
dependencies.

## Error handling

- Wi-Fi and MQTT reconnect loops with exponential backoff (cap 30 s).
- Any session error or timeout → log, drop session, back to idle; the
  device never wedges in a listening state.
- Heartbeat: optional subscribe to `athena/events/#` is out of scope;
  liveness is implied by MQTT keep-alive.

## Phasing

1. **Phase 1 — push-to-talk**: full round trip mic → runtime → speaker,
   triggered by the BOOT button. Proves wiring, I2S in/out, MQTT, and the
   protocol handling.
2. **Phase 2 — wake word**: `esp-sr` (WakeNet) added via the ESP-IDF
   component manager, called from Rust over FFI; mic frames feed WakeNet
   in idle, a detection acts as the trigger. Wake word model: **"Jarvis"**
   (stock WakeNet).

## Testing

- `session.rs` (and payload parsing helpers) unit-tested on the host:
  `cargo test` inside `satellite-esp32/` with `--no-default-features`
  style gating so esp-idf never builds on the host.
- Hardware verification is manual: `build.sh`, `espflash flash --monitor`,
  a live session against a running `serve`.
- Root workspace CI/tests untouched.

## Root repo changes

- `Cargo.toml`: add `satellite-esp32` to the `exclude` list with a
  comment (targets `xtensa-esp32s3-espidf`).
- `README.md`: one paragraph under the satellite protocol section linking
  to `satellite-esp32/README.md`.
- `PLAN.md` `## Notes`: record the `tts/meta` codec inconsistency
  (advertises opus, worker emits s16le PCM) for a future planning pass.

## Risks

- The espup/Xtensa toolchain must be installed on the build machine; the
  README documents `espup install` and sourcing `export-esp.sh`.
- `esp-sr` FFI from Rust (phase 2) is the least-trodden path; phase 1 is
  deliberately independent of it.
- Classic ESP32 (non-S3) is out of scope; RAM headroom and WakeNet models
  assume S3.
