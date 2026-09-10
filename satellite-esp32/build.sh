#!/usr/bin/env bash
# Cross-compile the firmware for xtensa-esp32s3-espidf. Requires the
# espup toolchain (see README.md).
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"
cargo build --release "$@"
