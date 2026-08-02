# ---------- build stage ----------
FROM rust:1.95-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2-dev libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# Host binary (release) + bundled WASM skills (each skill is its own
# workspace; build.sh drops the .wasm into /src/skills/). rust-toolchain.toml
# pins the wasm32-wasip1 target, so rustup installs it on the first cargo call
# below — no separate `rustup target add` needed.
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
# Bundled skills ship read-only under /app/skills (owned by athena so the
# entrypoint can copy them out on first boot). The [skills] dir the runtime
# actually loads from is /data/skills, on the writable /data volume — the
# web UI's upload_skill needs a location that survives image updates and
# isn't a root-owned image layer. New images do NOT overwrite skills already
# seeded into /data/skills; the web UI's bundled-skill picker can reinstall
# an updated bundled skill on demand.
COPY --from=build --chown=athena:athena /src/skills /app/skills
# Baked-in default config; docker-compose bind-mounts the user's copy over it.
COPY athena.docker.example.toml /app/athena.docker.toml
COPY --chown=athena:athena docker/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh
USER athena
WORKDIR /app
ENV HOME=/home/athena
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["serve", "--config", "/app/athena.docker.toml"]
