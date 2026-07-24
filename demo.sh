#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

# End-to-end local demo: MQTT broker → server → text-mode satellite client.
# Requires a broker on 127.0.0.1:1883 (e.g. `brew services start mosquitto`).

echo "🔨 Building server + client..."
cargo build -p athena-voice-cli -p athena-voice-client

echo "🔨 Building skills..."
./skills-smoke-test/build.sh
./skills-timer/build.sh
./skills-weather/build.sh

if ! nc -z 127.0.0.1 1883 2>/dev/null; then
    echo "❌ No MQTT broker on 127.0.0.1:1883 (try: brew services start mosquitto)" >&2
    exit 1
fi

echo "🚀 Starting server..."
./target/debug/athena-voice serve --config athena.local.toml &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT
sleep 6 # skills take a few seconds to load

echo "⏰ Asking the time..."
./target/debug/athena-voice-client --text "quelle heure est-il"

echo "🌤  Asking the weather (live open-meteo call)..."
./target/debug/athena-voice-client --text "météo à Strasbourg"

echo "⏲️  Setting a timer..."
./target/debug/athena-voice-client --text "minuteur 5 secondes"

echo "✅ Demo complete! (add --speak to any client call to hear the answers)"
