# Athena-Voice

Extensible Rust voice-assistant framework. Design: see [`docs/superpowers/specs/2026-07-10-athena-voice-design.md`](docs/superpowers/specs/2026-07-10-athena-voice-design.md).

## Development quickstart

```bash
cargo build --workspace
cargo nextest run --workspace
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --check
```

## Status

Under active development. See `docs/superpowers/plans/` for the current implementation roadmap.
