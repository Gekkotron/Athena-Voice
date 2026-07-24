# Athena-Voice

An extensible, self-hostable voice assistant framework in Rust. Speak (or
type) a question; WASM skills answer it with real data; a TTS engine speaks
the answer back. Everything is glued together over MQTT, so satellites
(clients) can be anything from a laptop CLI to an ESP32 with a microphone.

```text
mic/text ──▶ MQTT ──▶ VAD ─▶ STT (whisper.cpp) ─▶ intent matcher ─▶ WASM skill ─▶ answer
                                                       │ no match             │
                                                       ▼                      ▼
                                                  LLM fallback ──────────▶ TTS ─▶ 🔊
```

- **Skills** are sandboxed WASM plugins (Extism) with a capability-scoped
  host API: HTTP allowlists, MQTT topic ACLs, per-skill key-value storage
  with retention, scheduled events, local time, audio playback. Bundled:
  time, weather (Open-Meteo, no API key), timers, home automation (MQTT),
  and Jeedom sensors (read any sensor by voice via the Jeedom HTTP API).
- **Providers** are pluggable per stage: STT (whisper.cpp worker or fake),
  LLM (**optional** — `none` by default; unmatched questions get a spoken
  capabilities answer; opt into Ollama or any OpenAI-compatible endpoint,
  with a spoken apology when the backend is down), TTS (macOS `say`
  worker or fake). Real engines run as separate *worker*
  processes bridged over MQTT, so they can live on a different machine
  than the runtime.
- **Bilingual** out of the box (French + English); the session's locale
  picks patterns and answer language. Adding a language is patterns +
  phrases, no plumbing.

## Quickstart

```bash
git clone --recurse-submodules https://github.com/Gekkotron/Athena-Voice
cd Athena-Voice
./quickstart.sh          # add --yes to skip the model-download prompt
```

The script detects what your machine can do and starts the richest stack
available, then prints what to try. Modes:

| Mode | Needs | You get |
|---|---|---|
| `voice` | macOS + whisper model (~465 MB, offered by the script) | speak to it, it speaks back |
| `say` | macOS | type to it, it speaks back |
| `fake` | any OS with cargo + an MQTT broker (or docker) | type to it, it answers in text |

Then, in another terminal:

```bash
cargo run -p athena-voice-client -- --text "météo à Paris" --play
cargo run -p athena-voice-client -- --text "what time is it" --locale en
cargo run -p athena-voice-client -- --microphone --play --timeout-secs 30   # voice mode
cargo run -p athena-voice-client -- --text "météo à Paris" --timing         # latency breakdown
```

Linux note: the voice path currently lacks a TTS engine (`say` is macOS) —
a Piper engine for `athena-voice-tts-worker` is on the roadmap; everything
else (STT, skills, LLM, client audio) is cross-platform.

## Manual setup

Each piece is an ordinary process; `athena.local.toml` / `athena.say.toml` /
`athena.voice.toml` are ready-made configs whose headers document the exact
commands. In short: an MQTT broker, optionally
`cargo run -p athena-voice-stt-worker` and `-p athena-voice-tts-worker`,
then `cargo run -p athena-voice-cli -- serve --config <config>`.

## Enabling the Jeedom skill

The skill ships built (`quickstart.sh` and `./skills-jeedom/build.sh` both
produce `skills/jeedom.wasm`); it stays dormant until sensors are
configured. Three steps:

1. In Jeedom, note the **command id** of each sensor you want to expose
   (shown on the command's line in the equipment page) and an **API key**
   (Settings → System → Configuration → API).
2. Add to the config you serve with (e.g. `athena.voice.toml`):

   ```toml
   [skills.jeedom]
   http_allowlist = ["jeedom.local"]   # your box's hostname or IP
   config = { base_url = "http://jeedom.local", api_key = "YOUR_KEY", sensors = '[{"name":"température du salon","id":123,"unit":"degrés"},{"name":"humidité de la chambre","id":456,"unit":"pourcent"}]' }
   ```

   `name` is what you'll say (fuzzy-matched), `id` the Jeedom command id,
   `unit` is spoken after the value.
3. Restart the server, then: *« donne-moi la température du salon »* /
   *"give me the température du salon"*.

Skills in general follow the same recipe: build to `skills/<name>.wasm`,
add a `[skills.<name>]` section for its capabilities (HTTP/MQTT
allowlists, config), restart.

## Satellite protocol (write your own client)

Publish/subscribe under `athena/sat/<sat-id>/session/<uuid>/…`:

| Topic suffix | Direction | Payload |
|---|---|---|
| `start` | → runtime | `{"locale":"fr"}` |
| `audio` | → runtime | raw s16le mono 16 kHz PCM; **empty payload = end of utterance** |
| `text` | → runtime | UTF-8 utterance (bypasses STT) |
| `end` | → runtime | closes the session |
| `transcript` | ← runtime | `{"text","is_final"}` |
| `tts/text` | ← runtime | `{"text"}` — the answer as text |
| `tts` | ← runtime | audio chunks |
| `done` | ← runtime | `{"outcome"}` |

`athena/events/#` mirrors the runtime's full event bus for observability.

## Troubleshooting

- **Skills can reach the internet but not LAN devices (Jeedom, MQTT
  workers on another host) and the log says the HTTP call failed**: on
  macOS 15+, the app your terminal runs in needs the **Local Network**
  permission (System Settings → Privacy & Security → Local Network).
  Embedded terminals inside other apps often can't be granted it — run
  the stack from the plain Terminal.app instead. Diagnostic tell: `nc -z
  <host> <port>` succeeds while `curl` to the same host fails instantly.
- **Two servers answering everything twice**: only run one `serve` (or
  quickstart) at a time; any stray instance also subscribes to the
  satellite topics.

## Development

```bash
cargo check --workspace --all-targets
cargo nextest run --workspace
./SHOWCASE.sh            # skills + runtime integration suite
```

Skills live outside the host workspace (they target `wasm32-wasip1`); each
has a `build.sh` dropping its `.wasm` into `skills/`. See
`crates/athena-voice-skill-sdk` for the guest API and `skills-weather/` for
a full-featured example.

Roadmap and task backlog: [`PLAN.md`](PLAN.md). Feature status:
[`SHOWCASE.md`](SHOWCASE.md).
