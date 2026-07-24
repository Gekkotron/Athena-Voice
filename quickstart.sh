#!/usr/bin/env bash
set -euo pipefail

# Athena-Voice quickstart: fresh clone → answering assistant, one command.
#
#   ./quickstart.sh [--yes]
#
# Detects what your machine can do and starts the richest stack available:
#   voice  — whisper STT + say TTS (macOS with a whisper model)
#   say    — text in, real speech out (macOS without a whisper model)
#   fake   — text in, text out (any OS; no external engines)
#
# --yes: consent to the one large download (whisper model, ~465 MB) without
# prompting. Everything the script starts is stopped again on Ctrl-C.

cd "$(dirname "$0")"

YES=0
[[ "${1:-}" == "--yes" ]] && YES=1

BOLD=$(tput bold 2>/dev/null || true)
RESET=$(tput sgr0 2>/dev/null || true)
say_step() { echo "${BOLD}==> $*${RESET}"; }
die() { echo "❌ $*" >&2; exit 1; }

PIDS=()
cleanup() {
    trap - INT TERM EXIT
    if [[ ${#PIDS[@]} -gt 0 ]]; then
        say_step "stopping (${#PIDS[@]} processes)…"
        kill "${PIDS[@]}" 2>/dev/null || true
        wait "${PIDS[@]}" 2>/dev/null || true
    fi
}
trap cleanup INT TERM EXIT

# ---------------------------------------------------------------- prereqs
command -v cargo >/dev/null || die "cargo not found — install Rust: https://rustup.rs"

# ------------------------------------------------------------------ broker
say_step "MQTT broker"
if nc -z 127.0.0.1 1883 2>/dev/null; then
    echo "    broker already running on 127.0.0.1:1883 — using it"
elif command -v mosquitto >/dev/null; then
    echo "    starting mosquitto…"
    mosquitto -p 1883 >/tmp/athena-quickstart-mosquitto.log 2>&1 &
    PIDS+=($!)
    sleep 1
    nc -z 127.0.0.1 1883 2>/dev/null || die "mosquitto failed to start (see /tmp/athena-quickstart-mosquitto.log)"
elif command -v docker >/dev/null; then
    echo "    starting eclipse-mosquitto via docker…"
    docker run -d --rm --name athena-quickstart-mosquitto -p 1883:1883 \
        eclipse-mosquitto mosquitto -c /mosquitto-no-auth.conf >/dev/null
    trap 'docker stop athena-quickstart-mosquitto >/dev/null 2>&1 || true; cleanup' INT TERM EXIT
    sleep 2
    nc -z 127.0.0.1 1883 2>/dev/null || die "dockerized mosquitto failed to start"
else
    die "no MQTT broker: install mosquitto (brew install mosquitto / apt install mosquitto) or docker"
fi

# ------------------------------------------------------- speech capability
HAVE_SAY=0
command -v say >/dev/null && command -v afconvert >/dev/null && HAVE_SAY=1

MODEL=models/ggml-small.bin
HAVE_MODEL=0
if [[ -f "$MODEL" ]] && [[ $(wc -c <"$MODEL") -gt 1000000 ]]; then
    HAVE_MODEL=1
elif [[ "$HAVE_SAY" == 1 ]]; then
    echo
    echo "Speech-to-text needs a whisper model (~465 MB download, one time)."
    if [[ "$YES" == 1 ]]; then
        REPLY=y
    else
        read -r -p "Download models/ggml-small.bin now? [y/N] " REPLY
    fi
    if [[ "$REPLY" =~ ^[Yy]$ ]]; then
        say_step "downloading whisper model"
        curl -L --progress-bar -o "$MODEL" \
            https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
        HAVE_MODEL=1
    fi
fi

WHISPER_BIN=whisper.cpp/build/bin/whisper-cli
if [[ "$HAVE_MODEL" == 1 && ! -x "$WHISPER_BIN" ]]; then
    command -v cmake >/dev/null || die "cmake needed to build whisper.cpp (brew/apt install cmake)"
    say_step "building whisper.cpp (one time)"
    [[ -f whisper.cpp/CMakeLists.txt ]] || git submodule update --init whisper.cpp
    cmake -S whisper.cpp -B whisper.cpp/build -DCMAKE_BUILD_TYPE=Release >/dev/null
    cmake --build whisper.cpp/build -j --target whisper-cli >/dev/null
fi

# ------------------------------------------------------------ pick a mode
if [[ "$HAVE_MODEL" == 1 && "$HAVE_SAY" == 1 ]]; then
    MODE=voice CONFIG=athena.voice.toml
elif [[ "$HAVE_SAY" == 1 ]]; then
    MODE=say CONFIG=athena.say.toml
else
    MODE=fake CONFIG=athena.local.toml
fi
say_step "mode: $MODE (config: $CONFIG)"
[[ "$MODE" == fake ]] && echo "    (no macOS 'say' found — text-only until the Piper TTS engine lands)"

# ------------------------------------------------------------------ build
say_step "building skills"
./skills-smoke-test/build.sh >/dev/null
./skills-timer/build.sh >/dev/null
./skills-weather/build.sh >/dev/null
./skills-jeedom/build.sh >/dev/null
say_step "building binaries"
BINARIES=(-p athena-voice-cli -p athena-voice-client)
[[ "$MODE" != fake ]] && BINARIES+=(-p athena-voice-tts-worker)
[[ "$MODE" == voice ]] && BINARIES+=(-p athena-voice-stt-worker)
cargo build "${BINARIES[@]}"

# ------------------------------------------------------------------ start
# NB: process substitution (not `cmd | sed &`) so $! is the worker itself —
# otherwise cleanup would kill the log prefixer and orphan the worker.
prefix() { sed "s/^/[$1] /"; }

if [[ "$MODE" == voice ]]; then
    say_step "starting STT worker (whisper)"
    ./target/debug/athena-voice-stt-worker > >(prefix stt) 2>&1 &
    PIDS+=($!)
fi
if [[ "$MODE" != fake ]]; then
    say_step "starting TTS worker (say)"
    ./target/debug/athena-voice-tts-worker > >(prefix tts) 2>&1 &
    PIDS+=($!)
fi
say_step "starting server"
RUST_LOG=info ./target/debug/athena-voice serve --config "$CONFIG" > >(prefix serve) 2>&1 &
PIDS+=($!)

# Wait for the skills snapshot to load before declaring victory.
say_step "waiting for skills to load…"
for _ in $(seq 1 60); do
    if ./target/debug/athena-voice-client --host 127.0.0.1 --text "quelle heure est-il" --timeout-secs 5 >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

echo
echo "${BOLD}✅ Athena is up (mode: $MODE).${RESET} Try, in another terminal:"
echo
echo "    cargo run -p athena-voice-client -- --text \"météo à Paris\""
echo "    cargo run -p athena-voice-client -- --text \"what time is it\" --locale en"
if [[ "$MODE" != fake ]]; then
    echo "    cargo run -p athena-voice-client -- --text \"météo à Paris\" --play"
fi
if [[ "$MODE" == voice ]]; then
    echo "    cargo run -p athena-voice-client -- --microphone --play --timeout-secs 30"
fi
echo
echo "Ctrl-C stops everything this script started."
wait "${PIDS[@]}" 2>/dev/null || true