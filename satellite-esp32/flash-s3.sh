#!/usr/bin/env bash
# Build and flash the ESP32-S3 firmware INCLUDING the custom partition
# table and the esp-sr wake-word model ("Alexa"). Plain `cargo run` does
# not flash the model partition — use this script for the S3.
set -euo pipefail
cd "$(dirname "$0")"
[ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"

MCU=esp32s3 cargo build --release --target xtensa-esp32s3-espidf

BIN=target/xtensa-esp32s3-espidf/release/athena-satellite-esp32
BUILD_DIR=$(ls -d target/xtensa-esp32s3-espidf/release/build/esp-idf-sys-*/out/build 2>/dev/null | head -1)
SRMODELS="$BUILD_DIR/srmodels/srmodels.bin"
if [ ! -f "$SRMODELS" ]; then
    echo "srmodels.bin not found — did the esp-sr component build?" >&2
    exit 1
fi

# Flash the bootloader THIS build produced. espflash otherwise uses its
# own bundled one, which tracks a different ESP-IDF release (observed:
# a v6.1-beta bootloader booting a v5.3.3 app). The bootloader sets up
# the flash cache and PSRAM before handing over, so a mismatched pair
# runs correct code against wrongly-mapped memory — the app boots and
# then fails in unrelated places, e.g. "capacity overflow" inside
# format!. `boot:` in the monitor must report the same version as
# `app_init: ESP-IDF:`.
BOOTLOADER="$BUILD_DIR/bootloader/bootloader.bin"
if [ ! -f "$BOOTLOADER" ]; then
    echo "bootloader.bin not found in $BUILD_DIR — build first" >&2
    exit 1
fi

# Offset of the `model` partition in partitions_s3.csv.
MODEL_OFFSET=0x310000
# Must match CONFIG_ESPTOOLPY_FLASHSIZE in sdkconfig.defaults.esp32s3:
# espflash otherwise assumes 4 MB and patches a mismatched size into the
# image header, which reads back as corrupt flash at runtime.
FLASH_SIZE="${FLASH_SIZE:-8mb}"

# ERASE=1 ./flash-s3.sh wipes the chip first. Needed after changing the
# partition table (stale tables and NVS otherwise survive a plain flash)
# and the first thing to try if the firmware boots into garbage.
if [ "${ERASE:-0}" = "1" ]; then
    echo "erasing flash…"
    espflash erase-flash "$@"
fi

espflash flash --flash-size "$FLASH_SIZE" \
    --bootloader "$BOOTLOADER" \
    --partition-table partitions_s3.csv \
    "$BIN" "$@"
espflash write-bin "$MODEL_OFFSET" "$SRMODELS" "$@"
echo "Flashed app + wake model. Monitor with: espflash monitor"
