# Athena-Voice Showcase

## Project Status

| Plan | Feature | Status | Tests |
|------|---------|--------|-------|
| **1** | Barge-in on new final transcript | ✅ Done | ✅ |
| **2** | Skill hot-reload (dev mode) | ✅ Done | ✅ |
| **3** | Timer / reminder skill | ✅ Done | ✅ |
| **4** | WASM skill system | ✅ Done | ✅ |
| **5** | Socket-compatible runner for VAD → ASR → intent → TTS | ✅ Done | ✅ |
| **6** | Hermetic skills persistence & retention | ✅ Done | ✅ |
| **7** | Skill-driven audio playback | ✅ Done | ✅ |
| **8** | Skill-friendly INI-style config | ✅ Done | ✅ |
| **9** | Skill-local short-lived tmpfs | ✅ Done | ✅ |

## Implemented Skills

| Skill | Intents | Features |
|-------|---------|----------|
| `smoke-test` | `time.query` (real local time), `audio.play`, `audio.volume` | Strobes every host function, INI config |
| `timer` | `timer.set` (FR patterns) | State retention, tmp storage, MQTT events |
| `weather` | `weather.now`, `weather.tomorrow` (FR patterns) | Live open-meteo HTTP, geocoding cache |
| `home` | Light/switch control (FR patterns) | MQTT publish allowlist |
| `jeedom` | `jeedom.read` — sensor values by voice (FR + EN) | Jeedom HTTP API, fuzzy sensor names |

## Quick Start

```bash
# One-shot demo (broker + server + three example questions):
brew services start mosquitto   # once
./demo.sh

# Or by hand:
./skills-smoke-test/build.sh && ./skills-timer/build.sh && ./skills-weather/build.sh
cargo run -p athena-voice-cli -- serve --config athena.local.toml
# in another terminal (add --speak to hear the answer via macOS `say`):
cargo run -p athena-voice-client -- --text "météo à Strasbourg" --speak
```

The client is an MQTT "satellite": it publishes
`athena/sat/<sat>/session/<uuid>/{start,text,end}` and subscribes to the
session's `transcript` / `tts` / `done` topics (`--events` also mirrors
`athena/events/#`). The `text` topic injects a final transcript directly,
so no STT model is needed for testing.

## Next Plans

- **Plan 10**: Skill HTTP OAuth2 helper
- **Plan 11**: Skill fine-grained permission DSL
- **Plan 12**: Skill install manager (signed WASM)

## Architecture

```txt
▼ Pipeline
─────
VAD → ASR → Intent → Skill → TTS
│
├─ Hotword detector (always on)
├─ Wasm skill registry (hot reload)
├─ Persistent KV store + tmpfs
├─ INI/TOML config file support
├─ rodio audio sink (feature `audio`, cross-platform)
```