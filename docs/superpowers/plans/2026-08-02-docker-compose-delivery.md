# Docker Compose Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Installing Athena-Voice's assist profile on any Linux box becomes `cp .env.example .env` + `docker compose up -d` against a prebuilt GHCR image; updating becomes `./update.sh`.

**Architecture:** A multi-stage Dockerfile (rust:1.95 build → debian:bookworm-slim runtime with the `athena-voice` binary, bundled `.wasm` skills, and a `/data` volume), a compose file with an optional `broker`-profile mosquitto, and a CI job that publishes `ghcr.io/gekkotron/athena-voice` after the nextest job passes.

**Tech Stack:** Docker multi-stage builds, docker compose v2 (profiles), GitHub Actions (`docker/login-action`, `docker/metadata-action`, `docker/build-push-action`), existing figment `ATHENA__*` env overrides.

**Spec:** `docs/superpowers/specs/2026-08-02-docker-compose-delivery-design.md`

## Global Constraints

- Image name EXACTLY `ghcr.io/gekkotron/athena-voice`; tags `latest` + `sha-<short>` (metadata-action defaults). amd64 only.
- The container config is `athena.docker.toml`; broker settings ONLY via `ATHENA__MQTT__*` env vars (figment `ATHENA__` prefix, `__` nesting — already works, no Rust changes in this plan).
- Bundled skills in the image: smoke-test, weather, jeedom (NOT timer — it silently never rings under serve; NOT home — unbuildable, see final-review note in git history).
- `update.sh` shape (owner convention, adapted): `git pull --rebase` → `docker compose pull` → `docker compose up -d`.
- Local cargo commands need `RUSTUP_TOOLCHAIN=1.95` (this machine has an override); docker builds don't (the image toolchain is correct).
- Commit as Gekkotron on every commit: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit ...`, message ending with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Never commit `.env` (already gitignored). `.env.example` is committed.
- PLAN.md edits must obey its parser contract (checkbox at top of `## Done`, indented body, no blank lines inside the body).

## File Structure

- Create: `Dockerfile` — multi-stage image build
- Create: `.dockerignore` — keep build context small
- Create: `athena.docker.toml` — container profile (paths under /data, env-driven broker)
- Create: `docker-compose.yml` — athena + optional mosquitto (profile `broker`)
- Create: `docker/mosquitto.conf` — bundled broker config
- Create: `.env.example` — documented env template
- Create: `update.sh` — owner-convention updater
- Modify: `.github/workflows/ci.yml` — `docker` publish job (needs: test)
- Modify: `crates/athena-voice-cli/src/config.rs` — one parse test for the new profile
- Modify: `README.md` — "Run with Docker" subsection above the native run-book
- Modify: `PLAN.md` — Done entry at the end

---

### Task 1: Dockerfile, .dockerignore, athena.docker.toml

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`
- Create: `athena.docker.toml`
- Modify: `crates/athena-voice-cli/src/config.rs` (append one test)

**Interfaces:**
- Produces: an image whose entrypoint is `/app/athena-voice` with default command `serve --config /app/athena.docker.toml`; skills at `/app/skills/*.wasm`; writable `/data`. Task 2's compose file relies on: port 8080 exposed, `ATHENA__MQTT__HOST` env override, SQLite under `/data`.

- [ ] **Step 1: Write the failing config-parse test**

Append to the tests module in `crates/athena-voice-cli/src/config.rs` (mirror the existing `parses_assist_profile_toml` test exactly — read it first for the repo-root path helper):

```rust
#[test]
fn parses_docker_profile_toml() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let cfg = load(&repo_root.join("athena.docker.toml")).expect("docker profile parses");
    let assist = cfg.assist.expect("assist enabled in docker profile");
    assert!(assist.enabled);
    assert_eq!(assist.topic_prefix, "assist");
    assert!(cfg.storage.database_url.contains("/data/"));
    assert_eq!(cfg.skills.dir.as_deref(), Some(Path::new("/app/skills")));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `RUSTUP_TOOLCHAIN=1.95 cargo nextest run -p athena-voice-cli parses_docker 2>&1 | tail -5`
Expected: FAIL — file not found ("docker profile parses" panic).

- [ ] **Step 3: Create `athena.docker.toml`**

Base it on the real `athena.assist.toml` (read it first; keep the same `[skills.weather]` allowlist hosts and `[skills.jeedom]` entry verbatim), with these differences:

```toml
# Athena-Voice — container profile (used by the Docker image; see Dockerfile).
#
# Broker settings come from the environment, NOT this file:
#   ATHENA__MQTT__HOST=192.168.1.x     (required — placeholder below fails fast)
#   ATHENA__MQTT__USERNAME=...         (optional)
#   ATHENA__MQTT__PASSWORD=...         (optional)
# Any [section] key can be overridden as ATHENA__SECTION__KEY.

locales = ["fr", "en"]

[server]
host = "0.0.0.0"          # admin web UI, published by docker-compose on :8080
port = 8080
session_idle_secs = 120

[storage]
database_url = "sqlite:///data/athena-voice.db?mode=rwc"

[mqtt]
host = "set-ATHENA__MQTT__HOST"   # placeholder: unreachable on purpose → fail-fast names it
port = 1883
client_id = "athena-voice"

[providers]
stt = "fake"              # unused by the assist bridge
llm = "none"              # unmatched questions get a spoken capabilities answer
tts = "fake"              # unused by the assist bridge

[assist]
enabled = true
topic_prefix = "assist"
locale = "fr"

[skills]
dir = "/app/skills"
```

(then the `[skills.weather]` / `[skills.jeedom]` blocks copied from `athena.assist.toml`).

- [ ] **Step 4: Run the test to verify it passes**

Run: `RUSTUP_TOOLCHAIN=1.95 cargo nextest run -p athena-voice-cli 2>&1 | tail -3`
Expected: all PASS including `parses_docker_profile_toml`.

- [ ] **Step 5: Create `.dockerignore`**

```
target
skills-*/target
test-minimal/target
whisper.cpp
models
*.wav
docs
.superpowers
.github
.git
.env
*.db
coverage
lcov.info
```

(Note: `skills/` is NOT ignored as a source — it's gitignored output, but the image builds it fresh anyway; ignoring `target` dirs is what keeps the context under control.)

- [ ] **Step 6: Create `Dockerfile`**

```dockerfile
# ---------- build stage ----------
FROM rust:1.95-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2-dev libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-wasip1
WORKDIR /src
COPY . .
# Host binary (release) + bundled WASM skills (each skill is its own
# workspace; build.sh drops the .wasm into /src/skills/).
RUN cargo build --release -p athena-voice-cli
RUN ./skills-smoke-test/build.sh && ./skills-weather/build.sh && ./skills-jeedom/build.sh

# ---------- runtime stage ----------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 ca-certificates libasound2 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 athena \
    && mkdir -p /data && chown athena /data
COPY --from=build /src/target/release/athena-voice /app/athena-voice
COPY --from=build /src/skills /app/skills
COPY athena.docker.toml /app/athena.docker.toml
USER athena
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/app/athena-voice"]
CMD ["serve", "--config", "/app/athena.docker.toml"]
```

- [ ] **Step 7: Build the image locally and smoke-test it**

Run (expect 10-25 min on the first build — do not abort):
```bash
docker build -t athena-voice:local .
docker run --rm athena-voice:local serve --config /app/athena.docker.toml --dry-run
```
Expected: build succeeds; the dry-run prints the "ready" log line and exits 0 (dry-run returns before the broker probe, proving config + skills paths parse inside the container). Then verify skills made it in:
```bash
docker run --rm --entrypoint ls athena-voice:local /app/skills
```
Expected: `jeedom.wasm  smoke-test.wasm  weather.wasm` (names per each build.sh's copy line — verify against the scripts and adjust this assertion to reality).

If `docker` daemon isn't running, launch Docker Desktop first (`open -a Docker` and wait for `docker info` to succeed).

- [ ] **Step 8: Commit**

```bash
git add Dockerfile .dockerignore athena.docker.toml crates/athena-voice-cli/src/config.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Docker image: multi-stage build with bundled assist skills

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: compose file, .env.example, mosquitto config, update.sh — live verified

**Files:**
- Create: `docker-compose.yml`
- Create: `docker/mosquitto.conf`
- Create: `.env.example`
- Create: `update.sh` (executable)

**Interfaces:**
- Consumes: the `athena-voice:local` image from Task 1 (for local verification; the committed file references the GHCR image Task 3 publishes).

- [ ] **Step 1: Create `docker/mosquitto.conf`**

```
listener 1883
allow_anonymous true
persistence false
```

- [ ] **Step 2: Create `docker-compose.yml`**

```yaml
services:
  athena:
    image: ghcr.io/gekkotron/athena-voice:latest
    restart: unless-stopped
    env_file: .env
    ports:
      - "8080:8080"        # admin web UI
    volumes:
      - athena-data:/data  # SQLite: admin token + web-edited skill settings

  # Optional bundled broker for setups without an existing MQTT broker:
  #   set COMPOSE_PROFILES=broker and ATHENA__MQTT__HOST=mosquitto in .env
  mosquitto:
    image: eclipse-mosquitto:2
    profiles: ["broker"]
    restart: unless-stopped
    ports:
      - "1883:1883"
    volumes:
      - ./docker/mosquitto.conf:/mosquitto/config/mosquitto.conf:ro

volumes:
  athena-data:
```

No `depends_on` from athena to mosquitto: compose rejects depends_on onto an inactive-profile service; athena's fail-fast probe + `restart: unless-stopped` handles broker-not-yet-up. (This deviates from one spec line — note it in your report; the spec's error-handling intent is preserved by the restart loop.)

- [ ] **Step 3: Create `.env.example`**

```bash
# Copy me:  cp .env.example .env   — then pick ONE of the two blocks.

# ── Setup A: you already run an MQTT broker on your LAN (Jeedom box, Z2M…)
ATHENA__MQTT__HOST=192.168.1.2
#ATHENA__MQTT__USERNAME=athena
#ATHENA__MQTT__PASSWORD=change-me

# ── Setup B: no broker yet — run the bundled one (uncomment BOTH lines,
#             comment out Setup A's host)
#COMPOSE_PROFILES=broker
#ATHENA__MQTT__HOST=mosquitto
```

- [ ] **Step 4: Create `update.sh`** (then `chmod +x update.sh`)

```bash
#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

git pull --rebase
docker compose pull
docker compose up -d
```

- [ ] **Step 5: Validate compose syntax**

```bash
cp .env.example .env
docker compose config > /dev/null && echo compose-valid
docker compose --profile broker config | grep -q mosquitto && echo profile-valid
```
Expected: both echo lines. (If the `docker compose` plugin misbehaves on this machine, `docker-compose` v5 is installed standalone — same CLI.)

- [ ] **Step 6: Live verification with the local image and bundled broker**

Edit the LOCAL `.env` (never committed) to Setup B (`COMPOSE_PROFILES=broker`, `ATHENA__MQTT__HOST=mosquitto`), and temporarily point compose at the local image:
```bash
ATHENA_IMAGE_OVERRIDE=1 docker compose up -d --no-build 2>/dev/null || true
```
— actually simplest reliable override: run with an override file, do NOT edit the committed compose:
```bash
cat > /tmp/compose.local.yml <<'EOF'
services:
  athena:
    image: athena-voice:local
EOF
docker compose -f docker-compose.yml -f /tmp/compose.local.yml up -d
sleep 3
docker compose logs athena | grep -i "token\|ready\|assist bridge"
```
Expected: the one-time admin token block and "assist bridge enabled" in the logs. Then the round trip through the bundled broker (host port 1883):
```bash
mosquitto_sub -h 127.0.0.1 -p 1883 -t 'assist/#' -v > /tmp/docker-assist.log &
mosquitto_pub -h 127.0.0.1 -p 1883 -t assist/transcription/cli -m '{"text": "quelle heure est-il"}'
sleep 3 && cat /tmp/docker-assist.log
```
Expected: `{"status":"in progress"}` → `assist/tts/cli {"text":"il est …"}` → `{"status":"done"}`.

- [ ] **Step 7: Persistence check**

```bash
docker compose -f docker-compose.yml -f /tmp/compose.local.yml restart athena
sleep 3
docker compose logs --since 30s athena | grep -ci "admin ui token" || echo "no token reprint (GOOD)"
```
Expected: `no token reprint (GOOD)` — the token hash persisted in the `/data` volume.

- [ ] **Step 8: Tear down and commit**

```bash
docker compose -f docker-compose.yml -f /tmp/compose.local.yml --profile broker down
kill %1 2>/dev/null || true
git add docker-compose.yml docker/mosquitto.conf .env.example update.sh
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Docker compose delivery: optional bundled broker, .env template, update.sh

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```
(`.env` must NOT appear in `git status` — it's gitignored; verify before committing.)

---

### Task 3: CI publish job to GHCR

**Files:**
- Modify: `.github/workflows/ci.yml` (append one job)

**Interfaces:**
- Consumes: the existing `test` job (exact job key `test` in ci.yml).
- Produces: `ghcr.io/gekkotron/athena-voice:latest` + `:sha-<short>` on every main push that passes tests.

- [ ] **Step 1: Append the job to ci.yml**

```yaml
  docker:
    name: docker image
    runs-on: ubuntu-latest
    needs: test
    if: github.event_name == 'push' && github.ref == 'refs/heads/main'
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/metadata-action@v5
        id: meta
        with:
          images: ghcr.io/gekkotron/athena-voice
          tags: |
            type=raw,value=latest
            type=sha
      - uses: docker/build-push-action@v6
        with:
          context: .
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
```

Match the file's existing indentation (jobs are at 2 spaces). Validate: `ruby -e "require 'yaml'; YAML.load_file('.github/workflows/ci.yml'); puts 'valid'"`.

- [ ] **Step 2: Commit and push**

```bash
git add .github/workflows/ci.yml
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "CI: publish the assist image to GHCR after tests pass

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
```

- [ ] **Step 3: Watch the run — the docker job is gated on nextest**

```bash
gh run list --repo Gekkotron/Athena-Voice --limit 1
# poll until the docker job concludes (nextest ~15 min + image build ~15-25 min):
gh run view <run-id> --repo Gekkotron/Athena-Voice --json jobs --jq '.jobs[] | "\(.name): \(.conclusion)"'
```
Expected: `docker image: success` (after `nextest: success`). If nextest fails on the known `wasm::watcher` flake, the docker job is skipped — note it; a later push retries. If the docker job fails on permissions ("installation not allowed to Write organization package"), report it verbatim — the repo may need Settings → Actions → Workflow permissions → "Read and write", which is a MANUAL owner step; do not try to work around it.

- [ ] **Step 4: Verify the published image is pullable (authenticated)**

```bash
gh auth token | docker login ghcr.io -u Gekkotron --password-stdin
docker pull ghcr.io/gekkotron/athena-voice:latest
docker run --rm ghcr.io/gekkotron/athena-voice:latest serve --config /app/athena.docker.toml --dry-run
```
Expected: pull + dry-run succeed. Anonymous pull requires the package to be made public — a MANUAL owner step (GitHub → Gekkotron → Packages → athena-voice → Package settings → Change visibility → Public); record it as pending in your report.

---

### Task 4: README + PLAN.md, final gate

**Files:**
- Modify: `README.md`
- Modify: `PLAN.md`

- [ ] **Step 1: README — insert a "Run with Docker" subsection ABOVE the "Run on a Linux box (GEEKOM / home server)" section's manual steps**

Insert right after that section's intro paragraph (adapt placement so it reads naturally — Docker is the headline path, the native steps stay below under a "Native install (no Docker)" subheading):

```markdown
### Run with Docker (recommended)

No Rust, no packages, no compiling — a prebuilt image is published by CI:

    git clone https://github.com/Gekkotron/Athena-Voice && cd Athena-Voice
    cp .env.example .env      # point ATHENA__MQTT__HOST at your LAN broker
    docker compose up -d

No broker yet? Uncomment Setup B in `.env` (bundled mosquitto):

    docker compose --profile broker up -d

The one-time admin token prints in the logs on first start:

    docker compose logs athena     # look for the "Admin UI token" block

Updating:

    ./update.sh                    # git pull --rebase + compose pull + up -d

Data (admin token, web-edited skill settings) lives in the `athena-data`
volume and survives updates and restarts.
```

Check the real admin-token log wording in `crates/athena-voice-cli/src/serve.rs` (the `println!` block) and make the README's "look for" phrase match it.

- [ ] **Step 2: PLAN.md Done entry** (top of `## Done`, format contract: no blank lines in body)

```markdown
- [x] Docker Compose delivery with prebuilt GHCR image (2026-08-02)
      Multi-stage Dockerfile (rust:1.95 build → bookworm-slim runtime, non-root, /data volume) bundling the smoke-test/weather/jeedom skills and athena.docker.toml (broker via ATHENA__MQTT__* env). docker-compose.yml with optional broker-profile mosquitto, .env.example, update.sh (pull --rebase + compose pull + up -d). CI `docker` job publishes ghcr.io/gekkotron/athena-voice (latest + sha) after nextest passes. Live-verified locally: bundled-broker round trip and /data token persistence across restart. MANUAL owner step pending: make the GHCR package public. Spec: docs/superpowers/specs/2026-08-02-docker-compose-delivery-design.md.
```

- [ ] **Step 3: Full gate, commit, push**

```bash
RUSTUP_TOOLCHAIN=1.95 cargo nextest run -p athena-voice-cli 2>&1 | tail -3
git add README.md PLAN.md
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "README: Docker as the headline install path; PLAN update

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
gh run list --repo Gekkotron/Athena-Voice --limit 1
```
Expected: push ok; this push triggers CI again including another image publish — no need to wait for it beyond confirming the run started.

---

## Self-Review (done at authoring time)

- **Spec coverage:** Dockerfile/runtime/profile → T1; compose + profiles + .env + update.sh + live verification incl. persistence → T2; GHCR job + tags + manual visibility step → T3; README + PLAN → T4. Spec deviations called out inline: no `depends_on` onto the profiled broker (compose limitation; restart loop preserves intent), `.env` required by compose rather than optional (clear error + README covers it).
- **Placeholder scan:** the two "verify against reality" notes (wasm filenames in Step 7 of T1, admin-token log wording in T4) name exact files — deliberate verification, not gaps.
- **Type consistency:** image tag `athena-voice:local` used consistently in T1 step 7 and T2 step 6; job key `docker` matches T3 steps; profile name `broker` consistent across compose/.env/README.
