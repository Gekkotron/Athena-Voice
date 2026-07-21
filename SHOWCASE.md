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
| `smoke-test` | `time.query`, `audio.play`, `audio.volume` | Strobes every host function, INI config |
| `timer` | `timer.set` (FR patterns) | State retention, tmp storage, MQTT events |

## Quick Start

```bash
# Build all skills
./skills-smoke-test/build.sh
./skills-timer/build.sh

# Run runtime demo
cd crates/athena-voice-runtime
cargo run --example demo --no-default-features

# Test with event streams
socat - UNIX-CONNECT:/tmp/athena/events.sock
```

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
├─ PipeWire audio sink
```