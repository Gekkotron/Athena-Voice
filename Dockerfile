# ---------- build stage ----------
FROM rust:1.95-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
        libasound2-dev libssl-dev pkg-config cmake git \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# Host binaries (release): the server plus the two MQTT workers that give
# the image a full voice path (satellites stream audio; see the `voice`
# profile in docker-compose.yml). Bundled WASM skills come next — each
# skill is its own workspace and build.sh drops the .wasm into
# /src/skills/. rust-toolchain.toml pins the wasm32-wasip1 target, so
# rustup installs it on the first cargo call below — no separate
# `rustup target add` needed.
RUN cargo build --release \
        -p athena-voice-cli \
        -p athena-voice-stt-worker \
        -p athena-voice-tts-worker
RUN ./skills-smoke-test/build.sh && ./skills-weather/build.sh && ./skills-jeedom/build.sh

# whisper.cpp for the STT worker. Cloned here rather than shipped in the
# build context: the submodule is .dockerignore'd (it would bloat every
# build) and CI checkouts don't fetch submodules. Pinned to the same
# commit as the repo's submodule so image and dev tree agree.
ARG WHISPER_COMMIT=080bbbe85230f624f0b52127f1ae1218247989f9
RUN git clone https://github.com/ggerganov/whisper.cpp.git /whisper \
    && git -C /whisper checkout --quiet "$WHISPER_COMMIT" \
    && cmake -S /whisper -B /whisper/build -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /whisper/build -j "$(nproc)" --target whisper-cli

# ---------- runtime stage ----------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 ca-certificates libasound2 libgomp1 libstdc++6 \
        python3 python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 athena \
    && mkdir -p /data && chown athena /data
# Piper (portable TTS engine) in its own venv — Debian's PEP 668 marks
# the system Python as externally managed, so a venv is the supported
# install path. Voices are NOT vendored: mount them under /models
# (see "Voice on Linux (Piper)" in the README).
RUN python3 -m venv /opt/piper \
    && /opt/piper/bin/pip install --no-cache-dir piper-tts \
    && /opt/piper/bin/piper --help > /dev/null
COPY --from=build /src/target/release/athena-voice /app/athena-voice
COPY --from=build /src/target/release/athena-voice-stt-worker /app/athena-voice-stt-worker
COPY --from=build /src/target/release/athena-voice-tts-worker /app/athena-voice-tts-worker
COPY --from=build /whisper/build/bin/whisper-cli /app/whisper-cli
# Bundled skills ship read-only under /app/skills (owned by athena so the
# entrypoint can copy them out on first boot). The [skills] dir the runtime
# actually loads from is /data/skills, on the writable /data volume — the
# web UI's upload_skill needs a location that survives image updates and
# isn't a root-owned image layer. The entrypoint seeds them on first boot
# and auto-refreshes unmodified bundled skills on image updates (manifest-
# tracked); user-uploaded or user-replaced skills are never touched.
COPY --from=build --chown=athena:athena /src/skills /app/skills
# Baked-in default config; docker-compose mounts the user's copy over it.
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
