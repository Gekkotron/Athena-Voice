#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

# --autostash: local edits to tracked files (e.g. a tweaked athena.assist.toml)
# are stashed for the rebase and reapplied after, instead of aborting the update.
git pull --rebase --autostash
docker compose pull
docker compose up -d
