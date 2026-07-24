#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

echo "🔨 Building jeedom skill..."
cargo build --target wasm32-wasip1 --package skills-jeedom

mkdir -p ../skills
cp target/wasm32-wasip1/debug/skills_jeedom.wasm ../skills/jeedom.wasm
echo "✅ Copied to ../skills/jeedom.wasm"
