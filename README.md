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

## Web configuration

`serve` now hosts a small admin UI on `[server] host/port`
(default `http://127.0.0.1:8080`).

- **First start prints a one-time admin token** — save it; only its hash is
  stored. To reset it, delete the `admin_auth` row in the SQLite DB and
  restart.
- Configure skills (including the Jeedom API key and sensor list) in the
  browser: values land in the SQLite database, **never in TOML files**, and
  override `[skills.<name>]` TOML keys one by one.
- Enable/disable skills and upload new `.wasm` skills from the same page;
  changes apply live, no restart.
- To reach the UI from another machine, set `[server] host = "0.0.0.0"` —
  the token is still required.
- Any config value can also be overridden by environment variables:
  `ATHENA__SERVER__PORT=9090` (double underscore = nesting).

Skills can describe their settings by exporting `config_schema` (see
`skills-jeedom/src/lib.rs`); skills without it get a raw key/value editor.

## Run on a Linux box (GEEKOM / home server)

The assist profile answers **text** questions from a home-automation app
over MQTT — no audio stack, no whisper, no TTS engine on the server.

### Run with Docker (recommended)

Requires [Docker Engine and the compose plugin](https://docs.docker.com/engine/install/) (Debian/Ubuntu docs linked). The admin UI lands at `http://<server-ip>:9000` after `docker compose up -d` (host port 9000 because 8080 is often taken by tools like Zigbee2MQTT; override with `ATHENA_ADMIN_PORT=<port> docker compose up -d`).

No Rust, no packages, no compiling — a prebuilt image is published by CI:

    git clone https://github.com/Gekkotron/Athena-Voice && cd Athena-Voice
    cp athena.docker.example.toml athena.docker.toml   # your copy is gitignored
    # edit the [mqtt] block: your LAN broker's address (+ credentials if any)
    docker compose up -d

If you forget the `cp`, compose refuses to start with a clear
"file not found" error for `./athena.docker.toml` — copy the example and
`up -d` again. (Config changes need `docker compose up -d` to take effect —
the file is snapshotted into the container at creation, not live-mounted.)

No broker yet? Set `host = "mosquitto"` in the `[mqtt]` block and start
the bundled one (updates then need the same flag: `./update.sh --profile broker`):

    docker compose --profile broker up -d

The bundled `--profile broker` mosquitto allows anonymous access and is
published on the host's LAN interfaces — fine for a home LAN, not for a
host exposed beyond it.

The one-time admin token prints in the logs on first start:

    docker compose logs athena     # look for the "Admin UI token" block

The first `docker compose pull` needs the GHCR package to be **public**
(a one-time owner step in the package's GitHub settings) or a prior
`docker login ghcr.io`.

Updating:

    ./update.sh                    # git pull --rebase + compose pull + up -d

Data (admin token, web-edited skill settings) lives in the `athena-data`
volume and survives updates and restarts. Bundled skills are seeded into
that same volume on first boot; the admin UI can also upload or reinstall
skills there, and they persist across image updates. After an image update,
NEW bundled skills included in that image do not auto-appear — the volume
is seeded only on first boot; install them via the admin UI's
bundled-skill picker.

### Native install (no Docker)

    # once: Rust + a C toolchain
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    sudo apt install build-essential pkg-config libasound2-dev libssl-dev

`libasound2-dev` is required even though the assist profile itself doesn't
touch audio: the workspace builds `athena-voice-client`'s audio pieces
along with everything else, and that crate links ALSA on Linux.
`libssl-dev` is required by the MQTT stack's native-tls dependency (Linux
only; GitHub's CI runners preinstall it, a fresh box won't).

    git clone https://github.com/Gekkotron/Athena-Voice && cd Athena-Voice
    for s in skills-smoke-test skills-weather skills-jeedom; do ./$s/build.sh; done

    # your copy is gitignored — edits never block ./update.sh
    cp athena.assist.example.toml athena.assist.toml
    # point [mqtt] host at your LAN broker, then:
    cargo run --release -p athena-voice-cli -- serve --config athena.assist.toml

`rustup` reads `rust-toolchain.toml` and installs the pinned Rust version
automatically on first `cargo` invocation — `targets = ["wasm32-wasip1"]`
in that file makes it install the wasm target too (every `build.sh` above,
and even a plain `cargo run`, builds for it). Each `build.sh` drops its
`.wasm` straight into `skills/`.

`skills-timer` is deliberately left out of this build loop: setting a
timer works, but the expiry announcement needs scheduler wiring that
`serve` does not yet spawn in production (tracked in `PLAN.md`), so it
would never ring.

Broker credentials never live in the TOML: set `username` there and pass
`ATHENA__MQTT__PASSWORD=...` in the environment (any `[mqtt]` field can be
overridden as `ATHENA__MQTT__<FIELD>`). Subscriptions survive broker
restarts — the bridge resubscribes automatically on reconnect.

### Talking to it (DomoticApp protocol)

| Topic | Direction | Payload |
| --- | --- | --- |
| `assist/transcription/{device}` | app → Athena | `{"text": "quelle heure est-il"}` |
| `assist/tts/{device}` | Athena → app | `{"text": "il est 15 h 14"}` (one message per sentence) |
| `assist/llm/{device}/status` | Athena → app | `{"status": "in progress"}` then `{"status": "done"}` |

Try it without the app:

    mosquitto_sub -h <broker> -t 'assist/tts/#' -v &
    mosquitto_pub -h <broker> -t assist/transcription/cli \
      -m '{"text": "quelle heure est-il"}'

### Keep it running (systemd)

    # /etc/systemd/system/athena-voice.service
    [Unit]
    Description=Athena-Voice assist bridge
    Wants=network-online.target
    After=network-online.target mosquitto.service
    StartLimitIntervalSec=0

    [Service]
    WorkingDirectory=/home/<you>/Athena-Voice
    Environment=ATHENA__MQTT__PASSWORD=<secret>   # or use an EnvironmentFile
    ExecStart=/home/<you>/Athena-Voice/target/release/athena-voice serve --config athena.assist.toml
    Restart=on-failure
    RestartSec=5

    [Install]
    WantedBy=multi-user.target

## Enabling the Jeedom skill

1. In Jeedom: Settings → System → Configuration → API — copy (or create)
   an API key, ideally one restricted to the commands you want to expose.
2. In the Athena-Voice web UI (`http://127.0.0.1:8080`), open **jeedom**,
   fill in the Jeedom URL and API key, and **save**.
3. Click **Tester la connexion** — you should see the Jeedom version.
4. Click **Découvrir les capteurs**, tick the sensors you want by room,
   then **Ajouter la sélection** and save. Spoken names are pre-composed
   ("température du salon") and editable.
5. Ask by voice: "quelle est la température du salon", "quelle température
   dans la chambre", "toutes les températures", or for door/presence
   sensors "quelle est la porte du garage" → "la porte du garage est
   ouverte".

Existing installs: re-run `./skills-jeedom/build.sh` (the built wasm is
gitignored) to pick up the room/enumeration/binary phrasing above.

Skills in general follow the same recipe: build to `skills/<name>.wasm`
(or upload it straight from the web UI), then configure it there — skills
without a `config_schema` still get a raw key/value editor. A
`[skills.<name>]` TOML section remains available for non-secret defaults
(HTTP/MQTT allowlists, config) and is merged underneath whatever the UI
saves.

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
