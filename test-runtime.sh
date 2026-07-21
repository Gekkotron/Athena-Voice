#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/crates/athena-voice-runtime"

# 1. Build runtime
echo "🔨 Building runtime..."
cargo build

# 2. Run runtime integration tests
echo "🧪 Testing runtime..."
cargo nextest run --test audio_playback --test config_ini --test tmp_storage

# 3. Build smoke-test skill
echo "🔨 Building smoke-test skill..."
cd ../../skills-smoke-test
cargo build --target wasm32-wasip1

cd ..

# 4. Run demo (runtime only, no server/client)
echo "🚀 Testing skill dispatch..."
./target/debug/athena-voice-runtime-demo &
RUNTIME_PID=$!
sleep 3

# Dispatch test intents
echo -n '{"text": "quelle heure est-il", "final": true}' |
    socat - UNIX-CONNECT:/tmp/athena/events.sock
echo -n '{"text": "joue un son", "final": true}' |
    socat - UNIX-CONNECT:/tmp/athena/events.sock
echo -n '{"text": "minuteur 2 secondes", "final": true}' |
    socat - UNIX-CONNECT:/tmp/athena/events.sock

sleep 3
kill $RUNTIME_PID
echo "✅ Test complete!"