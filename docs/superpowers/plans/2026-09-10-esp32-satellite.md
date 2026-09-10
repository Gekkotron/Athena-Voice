# ESP32-S3 Voice Satellite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A standalone `satellite-esp32/` Rust firmware for an ESP32-S3 (INMP441 mic in, MAX98357A amp out) that speaks the existing `athena/sat/<id>/session/<uuid>/…` MQTT protocol, push-to-talk in this plan; on-device wake word is queued as a PLAN.md backlog task at the end.

**Architecture:** std Rust on ESP-IDF via `esp-idf-svc`. A hardware-free `session.rs` state machine (events in → commands out) carries all protocol logic and is unit-tested on the host; `mic.rs`/`speaker.rs`/`net.rs` are thin I2S/Wi-Fi/MQTT adapters; `main.rs` is an event loop that feeds the state machine and executes its commands.

**Tech Stack:** esp-idf-svc 0.51 (ESP-IDF v5.3), embuild, toml-cfg, uuid, serde_json, espup/espflash toolchain.

**Spec:** `docs/superpowers/specs/2026-09-10-esp32-satellite-design.md`

## Global Constraints

- The crate lives at `satellite-esp32/` and is **excluded** from the root workspace (`Cargo.toml` `exclude` list) — root `cargo check --workspace` must stay green and untouched by this crate.
- Package `authors = ["Gekkotron <60887050+Gekkotron@users.noreply.github.com>"]`, `license = "MIT"`. Never use the user's real name anywhere.
- Git commits: run with `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`, and end every commit message with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Audio to the runtime: raw **s16le mono 16 kHz** PCM; an **empty** `…/audio` payload ends the utterance.
- TTS from the runtime: `tts/meta` JSON (`{"sample_rate":24000,"channels":1,"frame_ms":20,"codec":"opus"}` — the codec field is wrong upstream; chunks are actually s16le PCM from the bundled worker). Play chunks as s16le at `sample_rate`.
- Session ids are hyphenated UUID v4 strings; satellite ids must match `^[a-z0-9-]{1,64}$`.
- Host tests must not build esp-idf: everything hardware lives behind the crate's default `hardware` feature; host tests run with `--no-default-features --target <host-triple>`.
- Secrets (`cfg.toml`) are gitignored; only `cfg.example.toml` is committed.

---

### Task 1: Xtensa toolchain setup

**Files:** none in the repo (machine setup only).

**Interfaces:**
- Consumes: nothing.
- Produces: a working `esp` rustup toolchain + `espflash` + `ldproxy` on PATH; `~/export-esp.sh` to source before builds.

- [ ] **Step 1: Install the tools**

```bash
cargo install espup espflash ldproxy
espup install
```

`espup install` downloads the Xtensa Rust toolchain (~1 GB, several minutes) and writes `~/export-esp.sh`.

- [ ] **Step 2: Verify**

Run: `rustup toolchain list | grep esp && command -v espflash ldproxy`
Expected: an `esp` toolchain line and both binary paths. No commit (nothing changed in the repo).

---

### Task 2: Scaffold the `satellite-esp32/` crate

**Files:**
- Create: `satellite-esp32/Cargo.toml`, `satellite-esp32/rust-toolchain.toml`, `satellite-esp32/.cargo/config.toml`, `satellite-esp32/sdkconfig.defaults`, `satellite-esp32/build.rs`, `satellite-esp32/src/main.rs`, `satellite-esp32/cfg.example.toml`, `satellite-esp32/.gitignore`, `satellite-esp32/build.sh`, `satellite-esp32/test.sh`
- Modify: root `Cargo.toml` (the `exclude` list, after the `test-minimal` entry)

**Interfaces:**
- Consumes: Task 1's toolchain.
- Produces: a crate that cross-compiles (`./build.sh`) and host-tests (`./test.sh`); the `hardware` default feature gate every later task builds on.

- [ ] **Step 1: Write the crate manifest and build plumbing**

`satellite-esp32/Cargo.toml`:

```toml
[package]
name = "athena-satellite-esp32"
version = "0.1.0"
edition = "2021"
rust-version = "1.77"
license = "MIT"
authors = ["Gekkotron <60887050+Gekkotron@users.noreply.github.com>"]

[[bin]]
name = "athena-satellite-esp32"
path = "src/main.rs"
harness = false

[features]
default = ["hardware"]
# Everything that needs ESP-IDF. Host tests run with --no-default-features.
hardware = ["dep:esp-idf-svc", "dep:toml-cfg"]

[dependencies]
log = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
esp-idf-svc = { version = "0.51", optional = true }
toml-cfg = { version = "0.2", optional = true }

[build-dependencies]
embuild = "0.33"
toml-cfg = "0.2"

[profile.release]
opt-level = "s"

[profile.dev]
debug = true
opt-level = "z"
```

`satellite-esp32/rust-toolchain.toml`:

```toml
[toolchain]
channel = "esp"
```

`satellite-esp32/.cargo/config.toml`:

```toml
[build]
target = "xtensa-esp32s3-espidf"

[target.xtensa-esp32s3-espidf]
linker = "ldproxy"
runner = "espflash flash --monitor"
rustflags = ["--cfg", "espidf_time64"]

[unstable]
build-std = ["std", "panic_abort"]

[env]
MCU = "esp32s3"
ESP_IDF_VERSION = "v5.3.3"
```

`satellite-esp32/sdkconfig.defaults`:

```
CONFIG_ESP_MAIN_TASK_STACK_SIZE=20000
CONFIG_FREERTOS_HZ=1000
```

`satellite-esp32/build.rs`:

```rust
fn main() {
    #[cfg(feature = "hardware")]
    embuild::espidf::sysenv::output();
}
```

(Note: `build.rs` features follow the crate's features — with `--no-default-features` the embuild call is compiled out and no ESP-IDF is required on the host.)

`satellite-esp32/.gitignore`:

```
/target
/.embuild
cfg.toml
```

`satellite-esp32/cfg.example.toml` (copied to `cfg.toml` by the user; keys must match the `toml-cfg` struct in Task 4):

```toml
[athena-satellite-esp32]
wifi_ssid = "MyNetwork"
wifi_pass = "hunter2"
mqtt_url = "mqtt://192.168.1.10:1883"
sat_id = "esp32-sat"
locale = "fr"
```

`satellite-esp32/build.sh` (mirrors the `skills-*/build.sh` convention):

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"
cargo build --release "$@"
```

`satellite-esp32/test.sh` (host tests, no ESP-IDF):

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
HOST_TRIPLE=$(rustc +stable -vV | sed -n 's/^host: //p')
cargo +stable test --no-default-features --target "$HOST_TRIPLE" "$@"
```

`chmod +x` both scripts.

- [ ] **Step 2: Minimal main.rs**

```rust
#[cfg(feature = "hardware")]
fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!("athena-satellite-esp32 boot");
}

#[cfg(not(feature = "hardware"))]
fn main() {}
```

- [ ] **Step 3: Exclude the crate from the root workspace**

In root `Cargo.toml`, append to the `exclude` array (keep the existing comment style):

```toml
    # Targets xtensa-esp32s3-espidf (ESP-IDF firmware); can't share the
    # workspace's native target. Build with satellite-esp32/build.sh.
    "satellite-esp32",
```

- [ ] **Step 4: Verify both build paths**

Run: `./satellite-esp32/build.sh` — expected: first build downloads ESP-IDF via embuild (slow), then compiles clean.
Run: `./satellite-esp32/test.sh` — expected: compiles, 0 tests, PASS.
Run: `cargo check --workspace` at root — expected: unchanged, green.

- [ ] **Step 5: Commit**

```bash
git add satellite-esp32 Cargo.toml
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: scaffold standalone ESP32-S3 firmware crate

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: `session.rs` protocol state machine (host-tested, TDD)

**Files:**
- Create: `satellite-esp32/src/session.rs`
- Modify: `satellite-esp32/src/main.rs` (add `mod session;`)
- Test: inline `#[cfg(test)] mod tests` in `session.rs`

**Interfaces:**
- Consumes: nothing hardware — pure std.
- Produces (used verbatim by Task 7):

```rust
pub const UTTERANCE_MAX_MS: u64 = 10_000;
pub const REPLY_TIMEOUT_MS: u64 = 30_000;

pub enum Input {
    Trigger,                                   // button press (later: wake word)
    MicFrame(Vec<u8>),                         // s16le mono 16 kHz bytes
    SilenceDetected,                           // end of utterance from mic.rs
    Inbound { topic: String, payload: Vec<u8> }, // any MQTT message received
    Tick,                                      // periodic, for timeouts
}

pub enum Command {
    Publish { topic: String, payload: Vec<u8> },
    ConfigureSpeaker { sample_rate: u32 },
    Play(Vec<u8>),
    SessionEnded,                              // back to idle (UI/LED hook)
}

#[derive(Debug, PartialEq)]
pub enum State { Idle, Streaming, AwaitingReply, Playing }

pub struct Session { /* sat_id, locale, state, sid, deadlines */ }

impl Session {
    pub fn new(sat_id: &str, locale: &str) -> Self;
    pub fn state(&self) -> &State;
    /// Five filters main.rs subscribes to once at connect (never the
    /// wildcard athena/sat/<id>/# — that would echo our own audio back).
    pub fn subscriptions(sat_id: &str) -> [String; 5];
    pub fn handle(&mut self, input: Input, now_ms: u64) -> Vec<Command>;
}
```

Behavior to implement (each is a test):
- `Trigger` in `Idle`: generate a UUID v4 sid, emit `Publish` of `{"locale":"<locale>"}` to `athena/sat/<sat>/session/<sid>/start`, go `Streaming`, set deadline `now + UTTERANCE_MAX_MS`.
- `MicFrame` in `Streaming`: `Publish` the bytes to `…/<sid>/audio`.
- `SilenceDetected` (or utterance deadline hit on `Tick`) in `Streaming`: `Publish` an **empty** payload to `…/<sid>/audio`, go `AwaitingReply`, deadline `now + REPLY_TIMEOUT_MS`.
- `Inbound` with topic suffix `tts/meta` for the current sid: parse `sample_rate` from JSON, emit `ConfigureSpeaker`, go `Playing`.
- `Inbound` suffix `tts`: emit `Play(payload)` (also valid straight from `AwaitingReply`).
- `Inbound` suffix `done`: emit `SessionEnded`, go `Idle`.
- `Inbound` for a different sid or unknown suffix: no commands.
- `Tick` past deadline in `AwaitingReply`/`Playing`: `SessionEnded`, go `Idle` (device never wedges).
- `Trigger` while not `Idle`: ignored.
- `subscriptions()` returns exactly the five response filters: `athena/sat/<id>/session/+/transcript`, `…/+/tts`, `…/+/tts/meta`, `…/+/tts/text`, `…/+/done`.

- [ ] **Step 1: Write the failing tests** (all of the bullets above; representative examples — write the full set):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sid_from_start_topic(cmds: &[Command]) -> String {
        let Command::Publish { topic, .. } = &cmds[0] else { panic!("expected publish") };
        topic.split('/').nth(4).unwrap().to_string()
    }

    #[test]
    fn trigger_starts_session_with_uuid_and_locale() {
        let mut s = Session::new("esp32-sat", "fr");
        let cmds = s.handle(Input::Trigger, 0);
        let Command::Publish { topic, payload } = &cmds[0] else { panic!() };
        assert!(topic.starts_with("athena/sat/esp32-sat/session/"));
        assert!(topic.ends_with("/start"));
        let sid = topic.split('/').nth(4).unwrap();
        assert!(uuid::Uuid::parse_str(sid).is_ok());
        assert_eq!(payload, br#"{"locale":"fr"}"#);
        assert_eq!(*s.state(), State::Streaming);
    }

    #[test]
    fn silence_publishes_empty_audio_and_awaits_reply() {
        let mut s = Session::new("esp32-sat", "fr");
        let sid = sid_from_start_topic(&s.handle(Input::Trigger, 0));
        let cmds = s.handle(Input::SilenceDetected, 1000);
        let Command::Publish { topic, payload } = &cmds[0] else { panic!() };
        assert_eq!(topic, &format!("athena/sat/esp32-sat/session/{sid}/audio"));
        assert!(payload.is_empty());
        assert_eq!(*s.state(), State::AwaitingReply);
    }

    #[test]
    fn done_ends_session() {
        let mut s = Session::new("esp32-sat", "fr");
        let sid = sid_from_start_topic(&s.handle(Input::Trigger, 0));
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/done"),
                payload: br#"{"outcome":"ok"}"#.to_vec(),
            },
            2000,
        );
        assert!(matches!(cmds[0], Command::SessionEnded));
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn reply_timeout_returns_to_idle() {
        let mut s = Session::new("esp32-sat", "fr");
        s.handle(Input::Trigger, 0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(Input::Tick, 1000 + REPLY_TIMEOUT_MS + 1);
        assert!(matches!(cmds[0], Command::SessionEnded));
        assert_eq!(*s.state(), State::Idle);
    }
}
```

Plus tests for: mic frame forwarding, utterance max deadline, `tts/meta` → `ConfigureSpeaker` + `Playing`, `tts` → `Play`, wrong-sid ignored, trigger-while-busy ignored, `subscriptions()` contents.

- [ ] **Step 2: Run to verify failure**

Run: `./satellite-esp32/test.sh`
Expected: FAIL — `session` module items unresolved.

- [ ] **Step 3: Implement `session.rs`** — the enums/struct exactly as in Interfaces; `handle` is a `match (state, input)`; store `sid: Option<String>` and `deadline_ms: Option<u64>`; inbound topics matched by string prefix `athena/sat/<sat>/session/<sid>/` then suffix.

- [ ] **Step 4: Run tests to verify pass**

Run: `./satellite-esp32/test.sh` — expected: all PASS.
Also run: `./satellite-esp32/build.sh` — the module must compile for the device target too.

- [ ] **Step 5: Commit**

```bash
git add satellite-esp32/src
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: host-tested MQTT session state machine

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: `config.rs` + `net.rs` (Wi-Fi and MQTT)

**Files:**
- Create: `satellite-esp32/src/config.rs`, `satellite-esp32/src/net.rs`
- Modify: `satellite-esp32/src/main.rs`

**Interfaces:**
- Consumes: `Session::subscriptions` (Task 3).
- Produces (used by Task 7):

```rust
// config.rs — all hardware-feature-gated except the pin constants:
#[toml_cfg::toml_config]              // #[cfg(feature = "hardware")]
pub struct Config {
    #[default("")]        wifi_ssid: &'static str,
    #[default("")]        wifi_pass: &'static str,
    #[default("mqtt://127.0.0.1:1883")] mqtt_url: &'static str,
    #[default("esp32-sat")] sat_id: &'static str,
    #[default("fr")]      locale: &'static str,
}
// Pins (plain consts, no gate): MIC_BCLK=4, MIC_WS=5, MIC_SD=6,
// SPK_BCLK=15, SPK_LRC=16, SPK_DIN=7, BUTTON=0.

// net.rs (all hardware-gated):
pub fn connect_wifi(modem: Modem, sysloop: EspSystemEventLoop) -> anyhow-free Result<BlockingWifi<EspWifi<'static>>, EspError>;
pub struct Mqtt { /* EspMqttClient */ }
impl Mqtt {
    /// Connects, subscribes to the five session filters, and forwards every
    /// inbound message into `tx` as (topic, payload).
    pub fn connect(url: &str, sat_id: &str,
                   tx: std::sync::mpsc::Sender<(String, Vec<u8>)>) -> Result<Self, EspError>;
    pub fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<(), EspError>;
}
```

- [ ] **Step 1: Implement `config.rs`** — the `toml_cfg` struct above (generates a `CONFIG` static from `cfg.toml` at build time) plus the pin constants. Gate the toml-cfg part with `#[cfg(feature = "hardware")]`.

- [ ] **Step 2: Implement `net.rs`** — standard `BlockingWifi` connect loop (retry every 5 s forever with `log::warn!` on failure — the device must boot headless); `EspMqttClient::new` with a callback that decodes `EventPayload::Received { topic, data, .. }` and sends into `tx`; on the `Connected` event (also fired after ESP-IDF's automatic reconnects) it (re-)subscribes to `Session::subscriptions(sat_id)` at QoS 1. `publish` uses QoS 0, non-retained (audio frames tolerate loss; `start` is small enough to keep simple at QoS 0 for v1).

- [ ] **Step 3: Wire into `main.rs`** — after boot: `connect_wifi`, `Mqtt::connect`, log "wifi + mqtt up", then sleep-loop. Copy `cfg.example.toml` to `cfg.toml` locally with real values (gitignored).

- [ ] **Step 4: Verify**

Run: `./satellite-esp32/build.sh` — expected: clean build.
Run: `./satellite-esp32/test.sh` — expected: still green (net/config are compiled out).
If hardware is on hand: `cd satellite-esp32 && cargo run --release` and watch the monitor for "wifi + mqtt up"; otherwise defer to Task 7's live check.

- [ ] **Step 5: Commit**

```bash
git add satellite-esp32/src
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: Wi-Fi + MQTT transport and toml-cfg config

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: `mic.rs` — INMP441 capture + silence detection (helpers TDD'd)

**Files:**
- Create: `satellite-esp32/src/mic.rs`
- Modify: `satellite-esp32/src/main.rs` (`mod mic;`)
- Test: inline `#[cfg(test)]` in `mic.rs`

**Interfaces:**
- Consumes: pin constants (Task 4).
- Produces (used by Task 7):

```rust
/// Host-testable, no hardware:
/// INMP441 delivers 24-bit samples left-justified in 32-bit slots.
/// Convert raw i2s bytes (little-endian i32) to s16le with a >>14 shift
/// (drops the low bits, keeps ~2 bits of headroom as gain).
pub fn convert_inmp441(raw: &[u8], out: &mut Vec<i16>);

pub struct SilenceTracker { /* threshold, run_ms, frame_ms */ }
impl SilenceTracker {
    /// threshold: RMS amplitude below which a frame counts as silence
    /// (default 500); silence_ms: how long silence must run (spec: 800).
    pub fn new(threshold: u32, silence_ms: u32, frame_ms: u32) -> Self;
    pub fn reset(&mut self);
    /// Push one s16 frame; returns true when end-of-utterance is reached.
    pub fn push(&mut self, frame: &[i16]) -> bool;
}

/// Hardware-gated: I2S std Philips RX, 16 kHz, 32-bit slots, mono (left).
pub struct Mic { /* I2sDriver rx */ }
impl Mic {
    pub fn new(i2s: I2S0, bclk: gpio pin 4, ws: pin 5, din: pin 6) -> Result<Self, EspError>;
    /// Blocking read of one ~20 ms frame, converted to s16le bytes.
    pub fn read_frame(&mut self, out: &mut Vec<u8>) -> Result<(), EspError>;
}
```

- [ ] **Step 1: Write failing tests for the two host-testable units**

```rust
#[test]
fn convert_shifts_32bit_slots_to_i16() {
    // one sample: 0x12345600 as le i32 → >>14 → 0x48D1 area; use a
    // round number instead: i32 value 1 << 20 → (1<<20)>>14 == 64
    let raw = (1i32 << 20).to_le_bytes();
    let mut out = Vec::new();
    convert_inmp441(&raw, &mut out);
    assert_eq!(out, vec![64i16]);
}

#[test]
fn tracker_fires_after_sustained_silence() {
    let mut t = SilenceTracker::new(500, 800, 20);
    let loud = vec![2000i16; 320];
    let quiet = vec![10i16; 320];
    assert!(!t.push(&loud));
    for _ in 0..39 { assert!(!t.push(&quiet)); } // 780 ms
    assert!(t.push(&quiet));                     // 800 ms reached
}

#[test]
fn loud_frame_resets_the_silence_run() {
    let mut t = SilenceTracker::new(500, 800, 20);
    let loud = vec![2000i16; 320];
    let quiet = vec![10i16; 320];
    for _ in 0..30 { t.push(&quiet); }
    t.push(&loud);
    for _ in 0..39 { assert!(!t.push(&quiet)); }
    assert!(t.push(&quiet));
}
```

- [ ] **Step 2: Run to verify failure** — `./satellite-esp32/test.sh`, expected FAIL.

- [ ] **Step 3: Implement** — `convert_inmp441` (chunk raw by 4, `i32::from_le_bytes`, `>> 14`, clamp to i16); `SilenceTracker` (integer RMS: `sqrt(sum(x²)/n)` via `u64` accumulation and `isqrt`, count consecutive silent frames); hardware `Mic` behind `#[cfg(feature = "hardware")]` using `esp_idf_svc::hal::i2s::I2sDriver::new_std_rx` with `StdConfig::philips(16000, DataBitWidth::Bits32)` and `SlotMode::Mono`, reading 640 raw bytes/frame → 320 samples → 20 ms.

- [ ] **Step 4: Verify** — `./satellite-esp32/test.sh` PASS; `./satellite-esp32/build.sh` clean.

- [ ] **Step 5: Commit**

```bash
git add satellite-esp32/src
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: INMP441 I2S capture with RMS silence tracker

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: `speaker.rs` — MAX98357A playback queue

**Files:**
- Create: `satellite-esp32/src/speaker.rs`
- Modify: `satellite-esp32/src/main.rs` (`mod speaker;`)

**Interfaces:**
- Consumes: pin constants (Task 4).
- Produces (used by Task 7; all hardware-gated):

```rust
pub struct Speaker { /* queue Sender + playback thread handle */ }
impl Speaker {
    /// Spawns a playback thread owning the I2S TX driver
    /// (std Philips, 16-bit mono, initial rate 24_000).
    pub fn new(i2s: I2S1, bclk: pin 15, lrc: pin 16, din: pin 7) -> Result<Self, EspError>;
    /// From tts/meta. Recreates the driver if the rate changed.
    pub fn set_sample_rate(&self, rate: u32);
    /// Enqueue one s16le chunk; the thread writes it to I2S in order.
    pub fn enqueue(&self, chunk: Vec<u8>);
}
```

Implementation notes: an `mpsc::channel<SpeakerMsg>` with `SpeakerMsg::{Rate(u32), Chunk(Vec<u8>)}`; the thread blocks on `recv`, rebuilds the `I2sDriver` on a `Rate` change, and `write_all`s chunks with a generous timeout. No host-testable logic here (it's all driver calls) — this task is compile-verified and live-verified in Task 7.

- [ ] **Step 1: Implement `speaker.rs`** as above.
- [ ] **Step 2: Verify** — `./satellite-esp32/build.sh` clean; `./satellite-esp32/test.sh` still green.
- [ ] **Step 3: Commit**

```bash
git add satellite-esp32/src
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: MAX98357A I2S playback queue

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: `main.rs` event loop — full push-to-talk round trip

**Files:**
- Modify: `satellite-esp32/src/main.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–6, exactly as declared there.
- Produces: the working phase-1 firmware.

- [ ] **Step 1: Implement the event loop**

Shape (hardware-gated `main`):

```rust
enum AppEvent { Button, Frame(Vec<u8>), UtteranceEnd, Mqtt(String, Vec<u8>), Tick }
```

- One `mpsc::channel::<AppEvent>()`.
- Button thread: `PinDriver::input(gpio0)` with pull-up, poll every 20 ms, debounce, send `Button` on press.
- Mic thread: owns `Mic` + `SilenceTracker` + an `Arc<AtomicBool>` "streaming" flag set by the main loop; while streaming it sends `Frame` per 20 ms read and `UtteranceEnd` once the tracker fires (then clears itself until re-armed).
- MQTT inbound: `Mqtt::connect`'s `tx` mapped into `Mqtt(topic, payload)` events.
- Ticker thread: send `Tick` every 500 ms.
- Main loop: translate `AppEvent` → `session::Input` (`Button→Trigger`, `Frame→MicFrame`, `UtteranceEnd→SilenceDetected`, `Mqtt→Inbound`, `Tick→Tick`), call `session.handle(input, now_ms)` (`now_ms` from `std::time::Instant` since boot), then execute commands: `Publish` → `mqtt.publish`; `ConfigureSpeaker` → `speaker.set_sample_rate`; `Play` → `speaker.enqueue`; `SessionEnded` → clear the streaming flag + `log::info!`. Entering `Streaming` sets the flag and resets the tracker.

- [ ] **Step 2: Verify builds and host tests** — `./satellite-esp32/build.sh` and `./satellite-esp32/test.sh` both green.

- [ ] **Step 3: Live round-trip check (requires the hardware + a running server)**

On the host: `cargo run --release -p athena-voice-cli -- serve --config athena.voice.toml` (or the user's usual serve config, with the MQTT broker reachable from the ESP32's Wi-Fi).
On the device: `cd satellite-esp32 && cargo run --release`, press BOOT, speak, expect the spoken answer on the speaker and `transcript`/`done` lines in the monitor log. If no hardware is attached to this machine, mark this step as pending-user in the final report — do not claim it verified.

- [ ] **Step 4: Commit**

```bash
git add satellite-esp32/src
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: push-to-talk event loop, full satellite round trip

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Documentation + upstream notes

**Files:**
- Create: `satellite-esp32/README.md`
- Modify: root `README.md` (after the "Satellite protocol" table), `PLAN.md` (`## Notes` section and `## Backlog`)

**Interfaces:**
- Consumes: everything shipped in Tasks 1–7.
- Produces: user-facing docs; the phase-2 backlog task for the orchestrator.

- [ ] **Step 1: Write `satellite-esp32/README.md`** containing: what it is (one paragraph); a wiring table (INMP441: VDD→3V3, GND→GND, SCK→GPIO4, WS→GPIO5, SD→GPIO6, L/R→GND; MAX98357A: VIN→5V, GND→GND, BCLK→GPIO15, LRC→GPIO16, DIN→GPIO7); toolchain setup (`cargo install espup espflash ldproxy && espup install`, then `. ~/export-esp.sh`); config (`cp cfg.example.toml cfg.toml`, edit); build/flash (`./build.sh`, `cargo run --release`); host tests (`./test.sh`); push-to-talk usage; a "Known limitation" note that TTS chunks are played as raw s16le because the runtime's `tts/meta` codec field is currently wrong (see PLAN.md note).

- [ ] **Step 2: Root `README.md`** — one short paragraph after the satellite-protocol section: a ready-made hardware satellite lives in `satellite-esp32/` (ESP32-S3 + INMP441 + MAX98357A), link to its README.

- [ ] **Step 3: `PLAN.md`** — under `## Notes` add the upstream inconsistency:

```
- `tts/meta` (crates/athena-voice-runtime/src/pipeline/sink.rs) hardcodes
  `codec: "opus"` but the bundled tts-worker emits raw s16le PCM chunks;
  satellites currently must assume s16le. Fix the meta (or actually encode
  opus) in a future task.
```

Under `## Backlog` add the phase-2 task, obeying the PLAN.md format contract exactly (top-level checkbox, contiguous indented body, no blank lines or nested bullets inside the body):

```
- [ ] satellite-esp32: on-device "Jarvis" wake word via esp-sr WakeNet
      Add the espressif/esp-sr ESP-IDF component to satellite-esp32 via
      esp-idf-sys extra_components with a bindings header, wrap WakeNet in
      src/wake.rs as WakeNet::detect(&[i16]) -> bool, select the stock
      "Jarvis" WakeNet9 model in sdkconfig (verify the exact Kconfig name in
      the esp-sr docs), add the model partition to the flash layout, and
      feed idle-state mic frames to it in main.rs so a detection acts like
      the BOOT button. Spec: docs/superpowers/specs/2026-09-10-esp32-satellite-design.md.
      Success criteria: (a) ./satellite-esp32/build.sh compiles with esp-sr;
      (b) ./satellite-esp32/test.sh still green; (c) saying "Jarvis" starts
      a session exactly like a button press (verified on hardware, or marked
      blocked pending hardware).
```

- [ ] **Step 4: Verify** — render/read the three files; confirm the PLAN.md task body has no blank lines and no nested `- ` bullets (parser contract).

- [ ] **Step 5: Commit**

```bash
git add satellite-esp32/README.md README.md PLAN.md
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "satellite-esp32: docs, wiring guide, wake-word backlog task

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```
