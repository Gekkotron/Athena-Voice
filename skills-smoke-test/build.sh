#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

echo "🔨 Building smoke-test skill..."
cargo build --target wasm32-wasip1 --package skills-smoke-test

mkdir -p ../skills
cp target/wasm32-wasip1/debug/skills_smoke_test.wasm ../skills/smoke-test.wasm
echo "✅ Copied to ../skills/smoke-test.wasm"