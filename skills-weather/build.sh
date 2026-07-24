#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

echo "🔨 Building weather skill..."
cargo build --target wasm32-wasip1 --package skills-weather

mkdir -p ../skills
cp target/wasm32-wasip1/debug/skills_weather.wasm ../skills/weather.wasm
echo "✅ Copied to ../skills/weather.wasm"
