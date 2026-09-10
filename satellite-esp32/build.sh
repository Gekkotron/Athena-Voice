#!/usr/bin/env bash
# Cross-compile the firmware. Requires the espup toolchain (see README.md).
#   ./build.sh                # classic ESP32 (WROOM) — the default
#   MCU=esp32s3 ./build.sh    # ESP32-S3
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"

MCU="${MCU:-esp32}"
case "$MCU" in
    esp32) TARGET=xtensa-esp32-espidf ;;
    esp32s3) TARGET=xtensa-esp32s3-espidf ;;
    *) echo "unsupported MCU '$MCU' (esp32 | esp32s3)" >&2; exit 1 ;;
esac
export MCU
cargo build --release --target "$TARGET" "$@"
