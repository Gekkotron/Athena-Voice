# athena-satellite-esp32

Firmware for an ESP32 voice satellite — classic ESP32 (WROOM) or ESP32-S3
— with an INMP441 I2S microphone and a MAX98357A I2S amplifier, speaking
Athena-Voice's MQTT satellite protocol
(`athena/sat/<sat-id>/session/<uuid>/…`, see the root README). Push a
button, speak, and the answer plays back on the speaker — the heavy
lifting (STT, skills, TTS) happens on the server running `serve`.

Current trigger is push-to-talk on the BOOT button; on-device wake word
("Jarvis", via esp-sr WakeNet) is planned — see `PLAN.md` in the repo
root.

## Wiring

Classic ESP32 (WROOM) — the default build. GPIO6–11 are wired to the
module's internal flash, so the satellite avoids them:

| INMP441 | ESP32 (WROOM) |     | MAX98357A | ESP32 (WROOM) |
|---------|---------------|-----|-----------|---------------|
| VDD     | 3V3           |     | VIN       | 5V            |
| GND     | GND           |     | GND       | GND           |
| SCK     | GPIO32        |     | BCLK      | GPIO27        |
| WS      | GPIO25        |     | LRC       | GPIO26        |
| SD      | GPIO33        |     | DIN       | GPIO22        |
| L/R     | GND           |     |           |               |

ESP32-S3 (`MCU=esp32s3`):

| INMP441 | ESP32-S3 |         | MAX98357A | ESP32-S3 |
|---------|----------|---------|-----------|----------|
| VDD     | 3V3      |         | VIN       | 5V       |
| GND     | GND      |         | GND       | GND      |
| SCK     | GPIO4    |         | BCLK      | GPIO15   |
| WS      | GPIO5    |         | LRC       | GPIO16   |
| SD      | GPIO6    |         | DIN       | GPIO7    |
| L/R     | GND      |         |           |          |

Per-chip pin bindings live in `wiring()` in `src/main.rs` (ESP-IDF pins
are typed objects) if your board needs different ones. On both chips the
BOOT button (GPIO0) is the push-to-talk trigger.

## Toolchain (one-time)

This crate targets Xtensa (`xtensa-esp32-espidf` by default,
`xtensa-esp32s3-espidf` with `MCU=esp32s3`) and is excluded from the
repo's Rust workspace. It needs Espressif's Rust toolchain:

```bash
cargo install espup espflash ldproxy
espup install          # ~1 GB; writes ~/export-esp.sh
```

`build.sh` sources `~/export-esp.sh` automatically. The first build also
downloads and compiles ESP-IDF (v5.3) via embuild — expect it to be slow
once.

## Configure

```bash
cp cfg.example.toml cfg.toml   # gitignored
$EDITOR cfg.toml               # Wi-Fi, broker URL, sat id, locale
```

Config is baked in at build time (toml-cfg): rebuild after editing.

## Build, flash, run

```bash
./build.sh                     # classic ESP32 (WROOM) — the default
MCU=esp32s3 ./build.sh         # ESP32-S3 instead
cargo run --release            # flash over USB + serial monitor (espflash)
```

(For an S3, flash with
`MCU=esp32s3 cargo run --release --target xtensa-esp32s3-espidf`.)

With the server up (`cargo run --release -p athena-voice-cli -- serve …`
on a host the ESP32 can reach), press BOOT, speak, and wait for the
answer. The monitor logs the session lifecycle.

## Host tests

```bash
./test.sh
```

Runs the hardware-free modules (session state machine, sample conversion,
silence tracking) on the host — no Xtensa toolchain or ESP-IDF needed.

## Known limitation

The runtime's `tts/meta` currently advertises `codec: "opus"` while the
bundled TTS worker actually publishes raw s16le PCM chunks; this firmware
plays chunks as s16le at the advertised `sample_rate`. Already tracked as
the "Honest audio format metadata in tts/meta" task in the repo root
`PLAN.md`.
