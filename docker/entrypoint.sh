#!/bin/sh
set -eu
mkdir -p /data/skills
# First boot: seed the volume with the bundled skills so the assistant
# answers out of the box; later boots never overwrite user-managed state.
if [ -z "$(ls -A /data/skills)" ]; then
    cp /app/skills/*.wasm /data/skills/
fi
exec /app/athena-voice "$@"
