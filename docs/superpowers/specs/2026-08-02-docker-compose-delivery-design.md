# Docker Compose delivery (prebuilt image) — design

Date: 2026-08-02
Status: approved by owner (sections reviewed 2026-08-02)

## Goal

Installing Athena-Voice on a home server (the owner's GEEKOM, or anyone's
Linux box) becomes: install Docker, clone, `cp .env.example .env`, edit one
line, `docker compose up -d`. No Rust toolchain, no apt packages, no
compilation on the target machine. Updating becomes `./update.sh`.

## Decisions (owner-confirmed)

- Delivery: **prebuilt multi-stage image published by GitHub Actions to
  GHCR** (`ghcr.io/gekkotron/athena-voice`), amd64 only for now (GEEKOM is
  Intel; arm64 is a follow-up).
- Compose ships an **optional bundled mosquitto** behind the `broker`
  profile; default assumes an existing LAN broker configured via `.env`.
- **`update.sh`** following the owner's cross-project convention
  (HttpMqttEnd2EndEncryption et al.), adapted for pull-based images:
  `git pull --rebase` → `docker compose pull` → `docker compose up -d`.

## Dockerfile (repo root, multi-stage)

- Build stage `rust:1.95-bookworm`: `rustup target add wasm32-wasip1`;
  `apt-get install libasound2-dev libssl-dev pkg-config`; build the
  workspace release binary (`athena-voice`) and every bundled skill via the
  `skills-*/build.sh` scripts (they drop `.wasm` into `skills/`).
- Runtime stage `debian:bookworm-slim`: `libssl3 ca-certificates
  libasound2` only; non-root user; `/app/athena-voice`, `/app/skills/*.wasm`,
  and a container profile `athena.docker.toml`; `VOLUME /data` (SQLite DB:
  admin token hash + web-edited skill settings persist).
- `athena.docker.toml`: same shape as `athena.assist.toml` but
  `database_url` under `/data`, `[skills] dir = "/app/skills"`,
  `[server] host = "0.0.0.0"`, `[mqtt] host` left to the
  `ATHENA__MQTT__HOST` env override (a placeholder value + comment).

## docker-compose.yml (repo root)

- Service `athena`: `image: ghcr.io/gekkotron/athena-voice:latest`,
  `restart: unless-stopped`, `env_file: .env` (delivers the `ATHENA__*`
  variables into the container; compose separately auto-reads the same
  `.env` for `COMPOSE_PROFILES`), port `8080:8080` (admin UI), volume
  `athena-data:/data`.
- Service `mosquitto` (`eclipse-mosquitto:2`), compose profile `broker`,
  minimal config (anonymous LAN listener on 1883) mounted from
  `docker/mosquitto.conf`, port `1883:1883`.
- `.env.example` (committed; `.env` gitignored): two documented blocks —
  "existing LAN broker" (`ATHENA__MQTT__HOST=192.168.1.x`, optional
  username/password) and "bundled broker" (`COMPOSE_PROFILES=broker`,
  `ATHENA__MQTT__HOST=mosquitto`).

## update.sh (repo root, executable)

```bash
#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
git pull --rebase
docker compose pull
docker compose up -d
```

Deviation from the sibling projects' `down`+`--build` shape is deliberate:
images are prebuilt, and skipping `down` avoids restarting an unchanged
assistant. `COMPOSE_PROFILES` in `.env` keeps the bundled-broker choice
sticky across updates.

## CI publish job

New `docker` job in `.github/workflows/ci.yml`: pushes to `main` only,
`needs: test` (nextest must pass first), `docker/build-push-action` with
`ghcr.io/gekkotron/athena-voice:latest` + `:sha-<short7>` tags, logged in
via the workflow's `GITHUB_TOKEN` (`packages: write` permission). One-time
MANUAL step for the owner: set the GHCR package visibility to public so
anonymous pulls work.

## README

New "Run with Docker (recommended for home servers)" subsection ABOVE the
native Linux run-book (which stays): clone, `.env`, `up -d`, where the
admin token appears (`docker compose logs athena`), `./update.sh` for
updates, both broker setups.

## Error handling

- Wrong `ATHENA__MQTT__HOST`: the existing fail-fast probe exits with an
  actionable message → visible in `docker compose logs`; compose restarts
  (no systemd start-limit latching).
- Broker profile race at boot: `depends_on: mosquitto` (profile-scoped)
  plus the probe's failure-restart loop.
- Missing `.env`: compose refuses to start with a clear "env file not
  found" error (see Implementation deviations below — this superseded the
  original assumption that compose would start anyway).

## Testing

- CI: the publish job's image build is the Dockerfile's regression test;
  it must build from a clean checkout.
- Local, before shipping: build the image on the dev Mac, run
  `docker compose --profile broker up`, drive a French time question
  through the bundled mosquitto with `mosquitto_pub`, assert the
  in-progress → answer → done sequence, restart the stack, and confirm the
  admin token is NOT reprinted (proves `/data` persistence).

## Out of scope

- arm64 image (follow-up if a Pi user appears).
- Publishing versioned release tags (only `latest` + sha for now).
- Containerizing the voice-mode workers (whisper/say) — assist profile only.

## Implementation deviations (recorded post-review)

- No `depends_on` onto the profiled mosquitto: Docker Compose rejects profile-scoped `depends_on`; the fail-fast probe in the connect loop plus the `restart: unless-stopped` policy cover boot ordering.
- Missing `.env`: When `.env` is absent, compose fails with a clear error message ("environment variable not set") rather than starting with placeholder values, making the cause more actionable than the spec originally claimed.

## Owner directive 2026-08-03: .env removed

The `.env`/`.env.example` pair is gone. User configuration for the Docker
path is a gitignored `athena.docker.toml` (copied from the tracked
`athena.docker.example.toml`) bind-mounted read-only over the image's baked
default — one config format across native and Docker modes. Consequences:
the bundled-broker choice moved from `COMPOSE_PROFILES` in `.env` to an
explicit `--profile broker` flag (`update.sh` now passes its arguments
through to docker compose), and `ATHENA__*` env overrides remain available
but optional.

- 2026-08-03 follow-up: the TOML is delivered via a compose `configs`
  file mount instead of a bind volume — a missing `./athena.docker.toml`
  now fails `up` with a clear error instead of Docker auto-creating a
  directory at the path (which bit the first real install). Config edits
  require `docker compose up -d` to recreate the container.
