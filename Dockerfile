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
