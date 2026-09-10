#!/usr/bin/env bash
# Host-side unit tests for the hardware-free modules (session state
# machine, audio helpers). Skips ESP-IDF entirely.
set -euo pipefail
cd "$(dirname "$0")"
HOST_TRIPLE=$(rustc +stable -vV | sed -n 's/^host: //p')
cargo +stable test --no-default-features --target "$HOST_TRIPLE" "$@"
