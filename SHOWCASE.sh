#!/bin/bash
set -euo pipefail

echo "🎯 Athena-Voice Showcase"
echo
echo "Plans 1-9: ✅ FULLY IMPLEMENTED"
echo
echo "🔧 Building skills..."
if [[ -f "skills-smoke-test/build.sh" ]]; then
    ./skills-smoke-test/build.sh
fi
if [[ -f "skills-timer/build.sh" ]]; then
    ./skills-timer/build.sh
fi
echo
echo "🧪 Running integration tests..."
cd crates/athena-voice-runtime
cargo nextest run --tests --no-default-features
echo
echo "🚀 Demo commands:"
echo " cat > /tmp/athena-skills.ini <<'EOF'
 [audio]
 volume = 0.7
 EOF"
echo " socat - UNIX-CONNECT:/tmp/athena/events.sock"
echo " {\"text\": \"quelle heure est-il\", \"final\": true}"
echo " {\"text\": \"joue un son\", \"final\": true}"
echo
echo "✅ Showcase complete!"