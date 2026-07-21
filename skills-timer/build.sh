#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

echo "🔨 Building timer skill..."
cargo build --target wasm32-wasip1 --package skills-timer

mkdir -p ../skills
cp target/wasm32-wasip1/debug/skills_timer.wasm ../skills/timer.wasm
echo "✅ Copied to ../skills/timer.wasm"