#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

# 1. Build server + client
echo "🔨 Building server/client..."
cargo build --package athena-voice-server --package athena-voice-client

# 2. Build skills
echo "🔨 Building skills..."
./skills-smoke-test/build.sh
./skills-timer/build.sh

# 3. Prepare directories
mkdir -p /tmp/athena/skills
mkdir -p skills

# 4. Start server
echo "🚀 Starting server..."
./target/debug/athena-voice-server &
SERVER_PID=$!
sleep 2

# 5. Test microphone → ASR → intent → TTS
function test_microphone() {
    echo "🎤 Say: 'Quelle heure est-il'"
    ./target/debug/athena-voice-client --microphone 2>/dev/null | \
    awk '/Transcript:/ {
        print "📝 Transcript:" $2 " (final:" ($4 == "true" ? "yes" : "no") ")"
    } /SkillResponse:/ {
        print "🔊 TTS:" substr($0, index($0,$2))
    }' &
    CLIENT_PID=$!
    sleep 8
    kill $CLIENT_PID 2>/dev/null || true
}

# 6. Test timer skill
echo "⏲️ Testing timer skill..."
echo -n '{"text": "minuteur 5 secondes", "final": true, "session": "00000000-0000-0000-0000-000000000000"}' |
    socat - UNIX-CONNECT:/tmp/athena/events.sock

# 7. Test INI config + tmp storage
echo "📁 Testing INI config + tmp storage..."
echo '[skills.smoke-test]
config_file = "/tmp/athena-skills.ini"' > /tmp/athena.toml
echo '[audio]
volume = 0.7' > /tmp/athena-skills.ini

pkill -f athena-voice-server || true
sleep 2
./target/debug/athena-voice-server --skills-dir ./skills &
SERVER_PID=$!
sleep 3

echo -n '{"text": "test tmp foo bar", "final": true, "session": "00000000-0000-0000-0000-000000000001"}' |
    socat - UNIX-CONNECT:/tmp/athena/events.sock

# 8. Cleanup
echo "🧹 Terminating server..."
kill $SERVER_PID 2>/dev/null || true
rm -rf /tmp/athena{,.toml,-skills.ini}
echo "✅ Demo complete!"