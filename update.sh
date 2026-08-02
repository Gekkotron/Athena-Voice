#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

git pull --rebase
docker compose pull
docker compose up -d
