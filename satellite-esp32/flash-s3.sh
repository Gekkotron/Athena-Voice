#!/usr/bin/env bash
# Build and flash the ESP32-S3 firmware INCLUDING the custom partition
# table and the esp-sr wake-word model ("Alexa"). Plain `cargo run` does
# not flash the model partition — use this script for the S3.
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"

MCU=esp32s3 cargo build --release --target xtensa-esp32s3-espidf

BIN=target/xtensa-esp32s3-espidf/release/athena-satellite-esp32
SRMODELS=$(ls target/xtensa-esp32s3-espidf/release/build/esp-idf-sys-*/out/build/srmodels/srmodels.bin 2>/dev/null | head -1)
if [ -z "$SRMODELS" ]; then
    echo "srmodels.bin not found — did the esp-sr component build?" >&2
    exit 1
fi

# Offset of the `model` partition in partitions_s3.csv.
MODEL_OFFSET=0x310000

espflash flash --partition-table partitions_s3.csv "$BIN" "$@"
espflash write-bin "$MODEL_OFFSET" "$SRMODELS" "$@"
echo "Flashed app + wake model. Monitor with: espflash monitor"
