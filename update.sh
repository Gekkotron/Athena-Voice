#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

# --autostash: local edits to tracked files are stashed for the rebase and
# reapplied after, instead of aborting the update. (User configs —
# athena.docker.toml, athena.assist.toml — are gitignored and never in the
# rebase's way.)
#
# Extra arguments are passed to docker compose, e.g.:
#   ./update.sh --profile broker
git pull --rebase --autostash
docker compose "$@" pull
docker compose "$@" up -d
