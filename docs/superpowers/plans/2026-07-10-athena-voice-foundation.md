# Athena-Voice — Plan 1: Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Cargo workspace, the type vocabulary crate (`athena-voice-core`), the SQLite storage crate (`athena-voice-storage`), and the CLI binary skeleton (`athena-voice-cli`) so that `athena-voice serve --dry-run` boots, loads config, opens the database, logs "ready", and exits cleanly on SIGTERM — with a full unit test suite green and CI wired.

**Architecture:** Rust 2024-edition workspace with `resolver = "2"`. Three crates in `crates/`. Types + traits in `athena-voice-core` (zero I/O). Storage in `athena-voice-storage` (`sqlx` + SQLite, migrations auto-applied, WAL mode). Binary in `athena-voice-cli` (`clap` for args, `figment` for TOML + env config, `tracing`/`tracing-subscriber` for structured logs). Tests via `cargo-nextest`. No MQTT, no actors, no providers, no WASM, no dashboard — those land in later plans.

**Tech Stack:** Rust 1.85+ (2024 edition), `tokio` 1.x (multi-thread runtime, feature-gated to just what's needed), `sqlx` 0.8 (SQLite feature), `clap` 4.x, `figment` 0.10, `tracing` + `tracing-subscriber` (JSON output), `thiserror` 2.x, `serde` + `serde_json`, `uuid` 1.x (v4), `bytes` 1.x, `async-trait` 0.1, `cargo-nextest`, `cargo-deny`, `cargo-llvm-cov`.

## Global Constraints

- **Rust edition:** 2024. `rust-toolchain.toml` pins stable Rust ≥ 1.85 (edition 2024 minimum).
- **Crate namespace:** every crate name prefixed `athena-voice-`. Binary artifact name is `athena-voice`.
- **Warnings:** `#![deny(warnings)]` at the root of every crate's `lib.rs` / `main.rs`. CI uses `-D warnings` on clippy.
- **Error handling:** `thiserror` in library crates; `anyhow` only in `athena-voice-cli::main`. Never `.unwrap()` outside `#[cfg(test)]` code.
- **Async runtime:** `tokio` multi-thread. Never call `.block_on` inside library code.
- **Config file:** TOML. Path: `--config <path>` flag, defaults to `./athena.toml`, env var override `ATHENA__` prefix per `figment`.
- **Logging:** `tracing`, JSON output on stdout. Level controlled by `RUST_LOG` (defaults to `info`).
- **Copyright/License:** MIT. Every file starts with no header (spdx in `Cargo.toml` `license` field is authoritative).
- **Test isolation:** every test uses `:memory:` SQLite (never a filesystem file). No shared global state.
- **Formatting:** `rustfmt` with defaults + `rustfmt.toml` overrides listed in Task 1.
- **CI:** GitHub Actions, single workflow at `.github/workflows/ci.yml` for this plan. Must pass on `linux-amd64` (arm64 added in later plans).

---

## File structure produced by this plan

```
athena-voice/
├── Cargo.toml                                        # workspace manifest
├── rust-toolchain.toml                                # pin rust version
├── .gitignore                                         # rust + target + editor
├── LICENSE                                            # MIT
├── README.md                                          # dev quickstart
├── deny.toml                                          # cargo-deny config
├── rustfmt.toml                                       # formatting rules
├── athena.example.toml                                # sample config
├── .cargo/
│   └── nextest.toml                                   # test runner profile
├── .github/workflows/
│   └── ci.yml                                         # fmt+clippy+test+llvm-cov
├── crates/
│   ├── athena-voice-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                                 # re-exports
│   │       ├── ids.rs                                 # SessionId, SatelliteId, Locale
│   │       ├── types.rs                               # AudioFrame, Transcript, Intent, Completion
│   │       ├── event.rs                               # Event, Outcome, Stage, FinishReason
│   │       ├── provider.rs                            # Stt / Llm / Tts traits
│   │       └── error.rs                               # CoreError + traits
│   ├── athena-voice-storage/
│   │   ├── Cargo.toml
│   │   ├── migrations/
│   │   │   └── 0001_initial.sql
│   │   └── src/
│   │       ├── lib.rs                                 # re-exports
│   │       ├── store.rs                               # Store trait
│   │       ├── sqlite.rs                              # SqliteStore
│   │       ├── models.rs                              # DB row types
│   │       └── error.rs                               # StoreError
│   └── athena-voice-cli/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                                # tokio::main entry point
│           ├── cli.rs                                 # clap args
│           ├── config.rs                              # figment config schema
│           ├── logging.rs                             # tracing subscriber init
│           └── serve.rs                               # serve subcommand implementation
```

---

## Task 1: Workspace scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `LICENSE`
- Create: `README.md`
- Create: `rustfmt.toml`

**Interfaces:**
- Consumes: nothing (first task)
- Produces: a compilable-but-empty workspace that `cargo check --workspace` accepts.

- [ ] **Step 1: Create `Cargo.toml` at the repo root**

```toml
[workspace]
resolver = "2"
members = [
    "crates/athena-voice-core",
    "crates/athena-voice-storage",
    "crates/athena-voice-cli",
]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "MIT"
authors = ["Gekkotron <60887050+Gekkotron@users.noreply.github.com>"]
repository = "https://github.com/Gekkotron/Athena-Voice"

[workspace.dependencies]
# core
async-trait = "0.1"
bytes = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }

# runtime
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "sync", "time", "fs"] }
tokio-util = { version = "0.7", features = ["rt"] }

# tracing
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# storage
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "macros", "migrate", "chrono"] }
chrono = { version = "0.4", default-features = false, features = ["serde", "clock"] }

# cli
clap = { version = "4", features = ["derive", "env"] }
figment = { version = "0.10", features = ["toml", "env"] }
anyhow = "1"

# dev only
assert_cmd = "2"
insta = { version = "1", features = ["json"] }
predicates = "3"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"

[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.85"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
profile = "minimal"
```

- [ ] **Step 3: Create `.gitignore`**

```gitignore
# Rust
/target
Cargo.lock.bak
**/*.rs.bk

# IDE
.idea/
.vscode/
*.iml

# OS
.DS_Store
Thumbs.db

# Env / secrets
.env
.env.local
athena.local.toml

# Coverage
/coverage
*.profraw
lcov.info
```

- [ ] **Step 4: Create `LICENSE`** (MIT)

```
MIT License

Copyright (c) 2026 Gekkotron

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 5: Create `README.md`**

```markdown
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
```

- [ ] **Step 6: Create `rustfmt.toml`**

```toml
edition = "2024"
max_width = 100
tab_spaces = 4
newline_style = "Unix"
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

- [ ] **Step 7: Verify workspace parses**

Run: `cargo check --workspace --offline`
Expected: workspace error mentioning missing member crates (they don't exist yet) — this is expected. Fixes on next task.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore LICENSE README.md rustfmt.toml
git commit -m "chore: workspace scaffolding"
```

---

## Task 2: `athena-voice-core` — Newtype identifiers

**Files:**
- Create: `crates/athena-voice-core/Cargo.toml`
- Create: `crates/athena-voice-core/src/lib.rs`
- Create: `crates/athena-voice-core/src/ids.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct SessionId(pub Uuid)` with `SessionId::new_v4() -> Self`, `Display`, `FromStr`, `Serialize`, `Deserialize`.
  - `pub struct SatelliteId(String)` with `SatelliteId::new(s: impl Into<String>) -> Result<Self, IdError>` validating `^[a-z0-9-]{1,64}$`.
  - `pub struct Locale(String)` with `Locale::new(s: impl Into<String>) -> Result<Self, IdError>` validating BCP 47-ish subset (`^[a-z]{2}(-[A-Z]{2})?$`).
  - `pub enum IdError { InvalidSatelliteId(String), InvalidLocale(String) }`.

- [ ] **Step 1: Create `crates/athena-voice-core/Cargo.toml`**

```toml
[package]
name = "athena-voice-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Type vocabulary and provider traits for Athena-Voice."

[dependencies]
async-trait = { workspace = true }
bytes = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
uuid = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create empty `src/lib.rs`**

```rust
#![deny(warnings)]
//! Athena-Voice core type vocabulary.

pub mod ids;
```

- [ ] **Step 3: Write failing tests in `src/ids.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_new_v4_is_unique() {
        let a = SessionId::new_v4();
        let b = SessionId::new_v4();
        assert_ne!(a, b);
    }

    #[test]
    fn session_id_roundtrip_via_display_fromstr() {
        let a = SessionId::new_v4();
        let s = a.to_string();
        let b: SessionId = s.parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let a = SessionId::new_v4();
        let json = serde_json::to_string(&a).unwrap();
        let b: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn satellite_id_accepts_valid() {
        assert!(SatelliteId::new("phone-01").is_ok());
        assert!(SatelliteId::new("a").is_ok());
        assert!(SatelliteId::new("phone-abc-123").is_ok());
    }

    #[test]
    fn satellite_id_rejects_invalid() {
        assert!(SatelliteId::new("").is_err());
        assert!(SatelliteId::new("Phone-01").is_err());
        assert!(SatelliteId::new("phone_01").is_err());
        assert!(SatelliteId::new("phone/01").is_err());
        assert!(SatelliteId::new(&"a".repeat(65)).is_err());
    }

    #[test]
    fn locale_accepts_valid() {
        assert!(Locale::new("fr").is_ok());
        assert!(Locale::new("en").is_ok());
        assert!(Locale::new("fr-FR").is_ok());
        assert!(Locale::new("en-US").is_ok());
    }

    #[test]
    fn locale_rejects_invalid() {
        assert!(Locale::new("").is_err());
        assert!(Locale::new("FR").is_err());
        assert!(Locale::new("french").is_err());
        assert!(Locale::new("fr-fr").is_err());
        assert!(Locale::new("fr_FR").is_err());
    }
}
```

- [ ] **Step 4: Run tests, verify they fail**

Run: `cargo test -p athena-voice-core --lib ids::tests -- --nocapture`
Expected: compile error — `SessionId` / `SatelliteId` / `Locale` don't exist yet.

- [ ] **Step 5: Implement in `src/ids.rs`** (above the `#[cfg(test)]` block)

```rust
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum IdError {
    #[error("invalid SatelliteId `{0}`: must match ^[a-z0-9-]{{1,64}}$")]
    InvalidSatelliteId(String),
    #[error("invalid Locale `{0}`: must match ^[a-z]{{2}}(-[A-Z]{{2}})?$")]
    InvalidLocale(String),
    #[error("invalid SessionId: {0}")]
    InvalidSessionId(#[from] uuid::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SessionId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SatelliteId(String);

impl SatelliteId {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.is_empty()
            || s.len() > 64
            || !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(IdError::InvalidSatelliteId(s));
        }
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SatelliteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for SatelliteId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<SatelliteId> for String {
    fn from(v: SatelliteId) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Locale(String);

impl Locale {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        let bytes = s.as_bytes();
        let ok = match bytes.len() {
            2 => bytes.iter().all(|b| b.is_ascii_lowercase()),
            5 => {
                bytes[0].is_ascii_lowercase()
                    && bytes[1].is_ascii_lowercase()
                    && bytes[2] == b'-'
                    && bytes[3].is_ascii_uppercase()
                    && bytes[4].is_ascii_uppercase()
            }
            _ => false,
        };
        if !ok {
            return Err(IdError::InvalidLocale(s));
        }
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Locale {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<Locale> for String {
    fn from(v: Locale) -> Self {
        v.0
    }
}
```

- [ ] **Step 6: Run tests, verify they pass**

Run: `cargo test -p athena-voice-core --lib ids::tests`
Expected: all 7 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-core
git commit -m "feat(core): SessionId, SatelliteId, Locale newtypes with validation"
```

---

## Task 3: `athena-voice-core` — Value types

**Files:**
- Modify: `crates/athena-voice-core/src/lib.rs` (add `pub mod types;`)
- Create: `crates/athena-voice-core/src/types.rs`

**Interfaces:**
- Consumes: `SessionId` from Task 2.
- Produces:
  - `pub struct AudioFrame { pub session: SessionId, pub seq: u32, pub pcm: Bytes }`
  - `pub struct Transcript { pub text: String, pub is_final: bool, pub confidence: Option<f32> }`
  - `pub struct Intent { pub name: String, pub slots: BTreeMap<String, serde_json::Value>, pub confidence: f32 }`
  - `pub struct Completion { pub text: String, pub finish: FinishReason }`
  - `pub enum FinishReason { Stop, Length, ContentFilter, Error }`

  All types `Debug + Clone + Serialize + Deserialize`. `AudioFrame` does NOT derive `PartialEq` (would require `Bytes` equality — done via `.pcm.as_ref() == other.pcm.as_ref()` in tests).

- [ ] **Step 1: Add module to `lib.rs`**

```rust
#![deny(warnings)]
//! Athena-Voice core type vocabulary.

pub mod ids;
pub mod types;
```

- [ ] **Step 2: Write failing tests in `src/types.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SessionId;
    use bytes::Bytes;

    #[test]
    fn audio_frame_serde_roundtrip() {
        let a = AudioFrame {
            session: SessionId::new_v4(),
            seq: 42,
            pcm: Bytes::from_static(&[1, 2, 3, 4]),
        };
        let json = serde_json::to_string(&a).unwrap();
        let b: AudioFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(a.session, b.session);
        assert_eq!(a.seq, b.seq);
        assert_eq!(a.pcm.as_ref(), b.pcm.as_ref());
    }

    #[test]
    fn transcript_serde_roundtrip_final() {
        let a = Transcript { text: "hello".into(), is_final: true, confidence: Some(0.95) };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"is_final\":true"));
        let b: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.is_final, b.is_final);
        assert_eq!(a.confidence, b.confidence);
    }

    #[test]
    fn transcript_serde_omits_none_confidence() {
        let a = Transcript { text: "hi".into(), is_final: false, confidence: None };
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("confidence"), "unexpected key present in {json}");
    }

    #[test]
    fn intent_serde_roundtrip_with_slots() {
        let mut slots = std::collections::BTreeMap::new();
        slots.insert("city".into(), serde_json::json!("Paris"));
        slots.insert("day".into(), serde_json::json!(1));
        let a = Intent { name: "weather.query".into(), slots, confidence: 0.87 };
        let json = serde_json::to_string(&a).unwrap();
        let b: Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(a.name, b.name);
        assert_eq!(a.slots, b.slots);
        assert!((a.confidence - b.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn completion_serde_roundtrip() {
        let a = Completion { text: "il fait beau".into(), finish: FinishReason::Stop };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"finish\":\"stop\""));
        let b: Completion = serde_json::from_str(&json).unwrap();
        assert_eq!(a.text, b.text);
        assert!(matches!(b.finish, FinishReason::Stop));
    }
}
```

- [ ] **Step 3: Run tests, verify compile-error failure**

Run: `cargo test -p athena-voice-core --lib types::tests`
Expected: compile error — types don't exist.

- [ ] **Step 4: Implement in `src/types.rs`**

```rust
use std::collections::BTreeMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::ids::SessionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub session: SessionId,
    pub seq: u32,
    #[serde(with = "bytes_serde")]
    pub pcm: Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub name: String,
    pub slots: BTreeMap<String, serde_json::Value>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub finish: FinishReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Error,
}

mod bytes_serde {
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(b: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        s.serialize_str(&STANDARD.encode(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let s = String::deserialize(d)?;
        STANDARD
            .decode(s.as_bytes())
            .map(Bytes::from)
            .map_err(D::Error::custom)
    }
}
```

Add to `crates/athena-voice-core/Cargo.toml` dependencies:
```toml
base64 = "0.22"
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-core --lib types::tests`
Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-core
git commit -m "feat(core): AudioFrame, Transcript, Intent, Completion value types"
```

---

## Task 4: `athena-voice-core` — Event enum + supporting enums

**Files:**
- Modify: `crates/athena-voice-core/src/lib.rs` (add `pub mod event;`)
- Create: `crates/athena-voice-core/src/event.rs`

**Interfaces:**
- Consumes: `SessionId`, `SatelliteId`, `Locale` (Task 2); `Intent` (Task 3).
- Produces:
  - `pub enum Event { … }` (see full variant list below), tagged with `#[serde(tag = "kind", rename_all = "snake_case")]`.
  - `pub enum Outcome { Ok, Error, Cancelled, Overloaded, Orphaned }`.
  - `pub enum Stage { Ingest, Vad, Stt, Router, Skill, Llm, Tts, Sink, Storage }`.

- [ ] **Step 1: Add module to `lib.rs`**

```rust
pub mod event;
```
(add below existing `pub mod` lines)

- [ ] **Step 2: Write failing tests in `src/event.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{Locale, SatelliteId, SessionId};

    #[test]
    fn event_session_started_tagged_serde() {
        let e = Event::SessionStarted {
            session: SessionId::new_v4(),
            satellite: SatelliteId::new("phone-01").unwrap(),
            locale: Locale::new("fr").unwrap(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"session_started\""));
        let round: Event = serde_json::from_str(&json).unwrap();
        assert!(matches!(round, Event::SessionStarted { .. }));
    }

    #[test]
    fn event_session_ended_carries_outcome() {
        let e = Event::SessionEnded {
            session: SessionId::new_v4(),
            outcome: Outcome::Ok,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"outcome\":\"ok\""));
    }

    #[test]
    fn event_provider_error_carries_stage() {
        let e = Event::ProviderError {
            session: SessionId::new_v4(),
            stage: Stage::Stt,
            error: "timeout".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"stage\":\"stt\""));
        assert!(json.contains("\"error\":\"timeout\""));
    }

    #[test]
    fn outcome_variants_snake_case() {
        assert_eq!(serde_json::to_string(&Outcome::Ok).unwrap(), "\"ok\"");
        assert_eq!(serde_json::to_string(&Outcome::Overloaded).unwrap(), "\"overloaded\"");
        assert_eq!(serde_json::to_string(&Outcome::Orphaned).unwrap(), "\"orphaned\"");
    }
}
```

- [ ] **Step 3: Run tests, verify fail**

Run: `cargo test -p athena-voice-core --lib event::tests`
Expected: compile errors.

- [ ] **Step 4: Implement in `src/event.rs`**

```rust
use serde::{Deserialize, Serialize};

use crate::ids::{Locale, SatelliteId, SessionId};
use crate::types::Intent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    },
    AudioFrameDropped {
        session: SessionId,
        seq: u32,
    },
    TranscriptPartial {
        session: SessionId,
        text: String,
    },
    TranscriptFinal {
        session: SessionId,
        text: String,
    },
    IntentMatched {
        session: SessionId,
        intent: Intent,
    },
    SkillInvoked {
        session: SessionId,
        skill: String,
    },
    SkillPanicked {
        session: SessionId,
        skill: String,
        reason: String,
    },
    LlmFallback {
        session: SessionId,
    },
    TtsChunk {
        session: SessionId,
        seq: u32,
        bytes_len: usize,
    },
    SessionEnded {
        session: SessionId,
        outcome: Outcome,
    },
    ProviderError {
        session: SessionId,
        stage: Stage,
        error: String,
    },
    CircuitOpened {
        stage: Stage,
        provider: String,
    },
    CircuitClosed {
        stage: Stage,
        provider: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
    Cancelled,
    Overloaded,
    Orphaned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Ingest,
    Vad,
    Stt,
    Router,
    Skill,
    Llm,
    Tts,
    Sink,
    Storage,
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-core --lib event::tests`
Expected: all 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-core
git commit -m "feat(core): Event enum + Outcome + Stage"
```

---

## Task 5: `athena-voice-core` — Provider traits

**Files:**
- Modify: `crates/athena-voice-core/src/lib.rs` (add `pub mod provider;`)
- Create: `crates/athena-voice-core/src/provider.rs`

**Interfaces:**
- Consumes: `AudioFrame`, `Transcript`, `Completion`, `Locale`, `SessionId` (Tasks 2–3).
- Produces:
  - `#[async_trait] pub trait Stt: Send + Sync { async fn transcribe(...) -> Result<TranscriptStream, Box<dyn Error + Send + Sync>>; fn name(&self) -> &'static str; }`
  - `#[async_trait] pub trait Llm: Send + Sync { async fn complete(...) -> Result<CompletionStream, ...>; fn name(&self) -> &'static str; }`
  - `#[async_trait] pub trait Tts: Send + Sync { async fn synthesize(...) -> Result<AudioStream, ...>; fn name(&self) -> &'static str; }`
  - `pub type TranscriptStream = Pin<Box<dyn Stream<Item = Result<Transcript, ...>> + Send>>`
  - `pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<String, ...>> + Send>>` (streaming tokens/deltas)
  - `pub type AudioStream = Pin<Box<dyn Stream<Item = Result<Bytes, ...>> + Send>>`

  Errors typed as `Box<dyn std::error::Error + Send + Sync + 'static>` at this layer — concrete `SttError`/`LlmError`/`TtsError` live in `athena-voice-providers` (later plan). This keeps `athena-voice-core` I/O-free but still expressive.

- [ ] **Step 1: Add `futures-core` dependency**

`crates/athena-voice-core/Cargo.toml`:
```toml
futures-core = "0.3"
```

- [ ] **Step 2: Add module to `lib.rs`**

```rust
pub mod provider;
```

- [ ] **Step 3: Write failing compile-check test in `src/provider.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_dyn_stt(_: &(dyn Stt)) {}
    fn _assert_dyn_llm(_: &(dyn Llm)) {}
    fn _assert_dyn_tts(_: &(dyn Tts)) {}

    #[test]
    fn traits_are_object_safe() {
        // The `_assert_*` fns above only compile if the traits are object-safe.
        // A passing `cargo build` implies success; this test just documents intent.
    }
}
```

- [ ] **Step 4: Verify fail**

Run: `cargo build -p athena-voice-core`
Expected: `Stt` / `Llm` / `Tts` unresolved.

- [ ] **Step 5: Implement in `src/provider.rs`**

```rust
use std::error::Error as StdError;
use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;

use crate::ids::{Locale, SessionId};
use crate::types::{AudioFrame, Transcript};

pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

pub type AudioFrameStream = Pin<Box<dyn Stream<Item = AudioFrame> + Send>>;
pub type TranscriptStream = Pin<Box<dyn Stream<Item = Result<Transcript, BoxError>> + Send>>;
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<String, BoxError>> + Send>>;
pub type AudioStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>;

#[async_trait]
pub trait Stt: Send + Sync {
    async fn transcribe(
        &self,
        session: SessionId,
        locale: Locale,
        audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError>;

    fn name(&self) -> &'static str;
}

#[async_trait]
pub trait Llm: Send + Sync {
    async fn complete(
        &self,
        session: SessionId,
        locale: Locale,
        prompt: String,
        history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError>;

    fn name(&self) -> &'static str;
}

#[async_trait]
pub trait Tts: Send + Sync {
    async fn synthesize(
        &self,
        session: SessionId,
        locale: Locale,
        text: String,
    ) -> Result<AudioStream, BoxError>;

    fn name(&self) -> &'static str;
}
```

- [ ] **Step 6: Run compile & test**

Run: `cargo test -p athena-voice-core --lib provider::tests`
Expected: 1 test passes.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-core
git commit -m "feat(core): Stt/Llm/Tts provider traits (object-safe)"
```

---

## Task 6: `athena-voice-core` — Errors

**Files:**
- Modify: `crates/athena-voice-core/src/lib.rs` (add `pub mod error;`)
- Create: `crates/athena-voice-core/src/error.rs`

**Interfaces:**
- Consumes: `Locale` (Task 2), `Stage` (Task 4).
- Produces:
  - `pub enum CoreError { InvalidId(#[from] IdError), Cancelled, Timeout { stage: Stage, ms: u64 } }`.
  - `impl CoreError { pub fn is_retryable(&self) -> bool; pub fn to_user_message(&self, locale: &Locale) -> String; }`
  - The `to_user_message` method returns a canned localised phrase for at least FR and EN; unknown locales fall back to EN.

- [ ] **Step 1: Add module**

```rust
pub mod error;
```

- [ ] **Step 2: Write failing tests in `src/error.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Stage;
    use crate::ids::{IdError, Locale};

    #[test]
    fn is_retryable_truth_table() {
        assert!(!CoreError::InvalidId(IdError::InvalidLocale("x".into())).is_retryable());
        assert!(!CoreError::Cancelled.is_retryable());
        assert!(CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.is_retryable());
    }

    #[test]
    fn user_message_fr() {
        let fr = Locale::new("fr").unwrap();
        let msg = CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.to_user_message(&fr);
        assert!(msg.contains("délai") || msg.contains("temps"), "got {msg}");
    }

    #[test]
    fn user_message_en() {
        let en = Locale::new("en").unwrap();
        let msg = CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.to_user_message(&en);
        assert!(msg.contains("timed out") || msg.contains("timeout"), "got {msg}");
    }

    #[test]
    fn user_message_unknown_locale_falls_back_to_en() {
        let ja = Locale::new("ja").unwrap();
        let msg = CoreError::Cancelled.to_user_message(&ja);
        // Fallback = English
        assert!(!msg.is_empty());
        assert!(msg.chars().all(|c| c.is_ascii()), "expected ASCII EN fallback, got: {msg}");
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-core --lib error::tests`
Expected: compile error.

- [ ] **Step 4: Implement in `src/error.rs`**

```rust
use thiserror::Error;

use crate::event::Stage;
use crate::ids::{IdError, Locale};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    InvalidId(#[from] IdError),

    #[error("cancelled")]
    Cancelled,

    #[error("stage {stage:?} timed out after {ms}ms")]
    Timeout { stage: Stage, ms: u64 },
}

impl CoreError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }

    #[must_use]
    pub fn to_user_message(&self, locale: &Locale) -> String {
        let lang = &locale.as_str()[..2]; // ignore region for now

        match (self, lang) {
            (Self::Timeout { .. }, "fr") => "Désolé, j'ai mis trop de temps à répondre.".into(),
            (Self::Timeout { .. }, _)   => "Sorry, that timed out.".into(),
            (Self::Cancelled, "fr")     => "Annulé.".into(),
            (Self::Cancelled, _)        => "Cancelled.".into(),
            (Self::InvalidId(_), "fr")  => "Identifiant invalide.".into(),
            (Self::InvalidId(_), _)     => "Invalid identifier.".into(),
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p athena-voice-core --lib error::tests`
Expected: 4 tests pass.

- [ ] **Step 6: Full crate check**

Run: `cargo test -p athena-voice-core`
Expected: all core tests pass.

Run: `cargo clippy -p athena-voice-core -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-core
git commit -m "feat(core): CoreError + is_retryable + to_user_message (fr, en)"
```

---

## Task 7: `athena-voice-storage` — Store trait skeleton

**Files:**
- Create: `crates/athena-voice-storage/Cargo.toml`
- Create: `crates/athena-voice-storage/src/lib.rs`
- Create: `crates/athena-voice-storage/src/store.rs`
- Create: `crates/athena-voice-storage/src/error.rs`
- Create: `crates/athena-voice-storage/src/models.rs`

**Interfaces:**
- Consumes: `SessionId`, `SatelliteId`, `Locale` (core Task 2); `Event`, `Outcome`, `Stage` (core Task 4).
- Produces:
  - `pub trait Store: Send + Sync + 'static` with async methods (see impl step below).
  - `pub enum StoreError` typed via `thiserror`, wraps `sqlx::Error`.
  - `pub struct SessionRow`, `EventRow`, `ErrorRow`, `SatelliteRow`, `SkillKvRow` — DB row types with `Debug + Clone + Serialize + Deserialize`.

- [ ] **Step 1: Create `crates/athena-voice-storage/Cargo.toml`**

```toml
[package]
name = "athena-voice-storage"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Persistence layer for Athena-Voice (SQLite default, Postgres feature)."

[dependencies]
async-trait = { workspace = true }
chrono = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sqlx = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }

athena-voice-core = { path = "../athena-voice-core" }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread"] }

[lints]
workspace = true
```

- [ ] **Step 2: Create `src/lib.rs`**

```rust
#![deny(warnings)]
//! Persistence layer for Athena-Voice.

pub mod error;
pub mod models;
pub mod store;

pub use error::StoreError;
pub use store::Store;
```

- [ ] **Step 3: Create `src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("json (de)serialization: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Db(sqlx::Error::Database(ref e)) if e.message().contains("SQLITE_BUSY")
        )
    }
}
```

- [ ] **Step 4: Create `src/models.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use athena_voice_core::event::Outcome;
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub session: SessionId,
    pub satellite: SatelliteId,
    pub locale: Locale,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub session: SessionId,
    pub kind: String,
    pub payload: serde_json::Value,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRow {
    pub id: i64,
    pub session: SessionId,
    pub stage: String,
    pub variant: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteRow {
    pub id: SatelliteId,
    pub api_key_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
}
```

- [ ] **Step 5: Create `src/store.rs`**

```rust
use async_trait::async_trait;

use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

use crate::error::StoreError;
use crate::models::{ErrorRow, EventRow, SatelliteRow, SessionRow};

#[async_trait]
pub trait Store: Send + Sync + 'static {
    async fn record_session(
        &self,
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    ) -> Result<(), StoreError>;

    async fn finalize_session(
        &self,
        session: SessionId,
        outcome: Outcome,
    ) -> Result<(), StoreError>;

    async fn get_session(&self, session: SessionId) -> Result<Option<SessionRow>, StoreError>;

    async fn append_event(&self, event: &Event) -> Result<(), StoreError>;

    async fn list_events_by_session(
        &self,
        session: SessionId,
        limit: u32,
    ) -> Result<Vec<EventRow>, StoreError>;

    async fn append_error(
        &self,
        session: SessionId,
        stage: Stage,
        variant: &str,
        message: &str,
    ) -> Result<(), StoreError>;

    async fn skill_kv_get(
        &self,
        skill: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StoreError>;

    async fn skill_kv_set(
        &self,
        skill: &str,
        key: &str,
        value: &[u8],
    ) -> Result<(), StoreError>;

    async fn provision_satellite(
        &self,
        id: SatelliteId,
        api_key_hash: &str,
    ) -> Result<(), StoreError>;

    async fn find_satellite(
        &self,
        id: &SatelliteId,
    ) -> Result<Option<SatelliteRow>, StoreError>;
}
```

- [ ] **Step 6: Verify compile**

Run: `cargo check -p athena-voice-storage`
Expected: compiles cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): Store trait + StoreError + row models"
```

---

## Task 8: `athena-voice-storage` — SQLite schema + migrations

**Files:**
- Create: `crates/athena-voice-storage/migrations/0001_initial.sql`
- Create: `crates/athena-voice-storage/tests/migrations.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: A migration file that creates the initial schema. Migration is `sqlx::migrate!("./migrations")`-loadable. Tables: `sessions`, `events`, `errors`, `satellites`, `skill_kv`.

- [ ] **Step 1: Create `migrations/0001_initial.sql`**

```sql
-- Athena-Voice initial schema (v1)

CREATE TABLE sessions (
    session       TEXT PRIMARY KEY NOT NULL,
    satellite     TEXT NOT NULL,
    locale        TEXT NOT NULL,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    outcome       TEXT
);

CREATE INDEX idx_sessions_satellite ON sessions(satellite);
CREATE INDEX idx_sessions_started_at ON sessions(started_at);

CREATE TABLE events (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    session   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    payload   TEXT NOT NULL,
    at        TEXT NOT NULL
);

CREATE INDEX idx_events_session ON events(session);
CREATE INDEX idx_events_kind ON events(kind);
CREATE INDEX idx_events_at ON events(at);

CREATE TABLE errors (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    session   TEXT NOT NULL,
    stage     TEXT NOT NULL,
    variant   TEXT NOT NULL,
    message   TEXT NOT NULL,
    at        TEXT NOT NULL
);

CREATE INDEX idx_errors_session ON errors(session);
CREATE INDEX idx_errors_stage ON errors(stage);

CREATE TABLE satellites (
    id             TEXT PRIMARY KEY NOT NULL,
    api_key_hash   TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    last_seen      TEXT
);

CREATE TABLE skill_kv (
    skill   TEXT NOT NULL,
    key     TEXT NOT NULL,
    value   BLOB NOT NULL,
    PRIMARY KEY (skill, key)
);
```

- [ ] **Step 2: Add sqlx-cli-free migration test in `tests/migrations.rs`**

```rust
//! Tests that the initial migration applies cleanly to an in-memory SQLite.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, migrate::Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn open_memory_pool() -> sqlx::SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

#[tokio::test]
async fn migrations_apply_cleanly() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.expect("migration failed");
}

#[tokio::test]
async fn migrations_create_expected_tables() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.unwrap();

    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>(0)).collect();
    assert_eq!(
        names,
        vec![
            "errors".to_string(),
            "events".to_string(),
            "satellites".to_string(),
            "sessions".to_string(),
            "skill_kv".to_string(),
        ]
    );
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    // Second run must be a no-op.
    MIGRATOR.run(&pool).await.unwrap();
}
```

- [ ] **Step 3: Run tests, verify pass**

Run: `cargo test -p athena-voice-storage --test migrations`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): initial SQLite schema + migration tests"
```

---

## Task 9: `athena-voice-storage` — SqliteStore constructor

**Files:**
- Create: `crates/athena-voice-storage/src/sqlite.rs`
- Modify: `crates/athena-voice-storage/src/lib.rs` (add `pub mod sqlite;` + `pub use sqlite::SqliteStore;`)

**Interfaces:**
- Consumes: `Store` trait (Task 7), migration file (Task 8).
- Produces:
  - `pub struct SqliteStore { pool: sqlx::SqlitePool }`.
  - `impl SqliteStore { pub async fn open(url: &str) -> Result<Self, StoreError> }` — opens pool, sets WAL, runs migrations.
  - `impl SqliteStore { pub fn pool(&self) -> &sqlx::SqlitePool }` — for advanced uses & tests.
  - **Does not yet implement `Store` methods** — those come in Tasks 10–15. The struct exists; trait `impl` blocks are empty stubs that will fill in.

- [ ] **Step 1: Update `src/lib.rs`**

```rust
pub mod sqlite;
pub use sqlite::SqliteStore;
```

- [ ] **Step 2: Write failing test in `src/sqlite.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_memory_succeeds() {
        let store = SqliteStore::open("sqlite::memory:").await.unwrap();
        // The pool works and migrations have run:
        sqlx::query("SELECT COUNT(*) FROM sessions")
            .fetch_one(store.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn open_bad_url_returns_error() {
        let res = SqliteStore::open("not-a-url").await;
        assert!(res.is_err());
    }
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-storage --lib sqlite::tests`
Expected: compile error — `SqliteStore` doesn't exist.

- [ ] **Step 4: Implement in `src/sqlite.rs`**

```rust
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::error::StoreError;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub struct SqliteStore {
    pool: sqlx::SqlitePool,
}

impl SqliteStore {
    /// Opens a SQLite database at `url` (e.g. `"sqlite:./athena.db"` or `"sqlite::memory:"`),
    /// enables WAL mode, and applies migrations.
    pub async fn open(url: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(|e| StoreError::Db(sqlx::Error::Configuration(e.into())))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        MIGRATOR.run(&pool).await?;

        Ok(Self { pool })
    }

    #[must_use]
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p athena-voice-storage --lib sqlite::tests`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): SqliteStore::open with WAL + auto-migrate"
```

---

## Task 10: `athena-voice-storage` — Session methods

**Files:**
- Modify: `crates/athena-voice-storage/src/sqlite.rs`
- Create: `crates/athena-voice-storage/tests/sessions.rs`

**Interfaces:**
- Consumes: `SqliteStore` (Task 9), `Store` trait (Task 7).
- Produces: `impl Store for SqliteStore` — `record_session`, `finalize_session`, `get_session`.

- [ ] **Step 1: Write failing tests in `tests/sessions.rs`**

```rust
use athena_voice_core::event::Outcome;
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn record_and_get_session() {
    let s = store().await;
    let sid = SessionId::new_v4();
    let sat = SatelliteId::new("phone-01").unwrap();
    let loc = Locale::new("fr").unwrap();

    s.record_session(sid, sat.clone(), loc.clone()).await.unwrap();

    let row = s.get_session(sid).await.unwrap().expect("session row");
    assert_eq!(row.session, sid);
    assert_eq!(row.satellite, sat);
    assert_eq!(row.locale, loc);
    assert!(row.ended_at.is_none());
    assert!(row.outcome.is_none());
}

#[tokio::test]
async fn finalize_updates_outcome_and_ended_at() {
    let s = store().await;
    let sid = SessionId::new_v4();
    s.record_session(sid, SatelliteId::new("phone-01").unwrap(), Locale::new("en").unwrap())
        .await
        .unwrap();

    s.finalize_session(sid, Outcome::Ok).await.unwrap();

    let row = s.get_session(sid).await.unwrap().unwrap();
    assert!(row.ended_at.is_some());
    assert_eq!(row.outcome, Some(Outcome::Ok));
}

#[tokio::test]
async fn get_session_missing_returns_none() {
    let s = store().await;
    let sid = SessionId::new_v4();
    assert!(s.get_session(sid).await.unwrap().is_none());
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-storage --test sessions`
Expected: compile error — `impl Store for SqliteStore` missing.

- [ ] **Step 3: Implement — add to `src/sqlite.rs`**

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

use crate::models::{ErrorRow, EventRow, SatelliteRow, SessionRow};
use crate::store::Store;

#[async_trait]
impl Store for SqliteStore {
    async fn record_session(
        &self,
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (session, satellite, locale, started_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(session.to_string())
        .bind(satellite.as_str())
        .bind(locale.as_str())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn finalize_session(
        &self,
        session: SessionId,
        outcome: Outcome,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        let outcome_str = serde_json::to_value(outcome)?
            .as_str()
            .unwrap()
            .to_string();
        let n = sqlx::query(
            "UPDATE sessions SET ended_at = ?1, outcome = ?2 WHERE session = ?3",
        )
        .bind(now)
        .bind(outcome_str)
        .bind(session.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound(format!("session {session}")));
        }
        Ok(())
    }

    async fn get_session(&self, session: SessionId) -> Result<Option<SessionRow>, StoreError> {
        let row_opt: Option<(String, String, String, String, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT session, satellite, locale, started_at, ended_at, outcome \
                 FROM sessions WHERE session = ?1",
            )
            .bind(session.to_string())
            .fetch_optional(&self.pool)
            .await?;
        let Some((s, sat, loc, started, ended, outcome)) = row_opt else {
            return Ok(None);
        };
        Ok(Some(SessionRow {
            session: s.parse().map_err(|e: athena_voice_core::ids::IdError| {
                StoreError::Db(sqlx::Error::Decode(Box::new(e)))
            })?,
            satellite: SatelliteId::new(sat).map_err(|e| {
                StoreError::Db(sqlx::Error::Decode(Box::new(e)))
            })?,
            locale: Locale::new(loc).map_err(|e| {
                StoreError::Db(sqlx::Error::Decode(Box::new(e)))
            })?,
            started_at: DateTime::parse_from_rfc3339(&started)
                .map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?
                .with_timezone(&Utc),
            ended_at: ended
                .map(|s| DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)))
                .transpose()
                .map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?,
            outcome: outcome
                .map(|s| serde_json::from_value::<Outcome>(serde_json::Value::String(s)))
                .transpose()?,
        }))
    }

    async fn append_event(&self, _event: &Event) -> Result<(), StoreError> {
        // filled in Task 11
        unimplemented!("Task 11")
    }

    async fn list_events_by_session(
        &self,
        _session: SessionId,
        _limit: u32,
    ) -> Result<Vec<EventRow>, StoreError> {
        unimplemented!("Task 11")
    }

    async fn append_error(
        &self,
        _session: SessionId,
        _stage: Stage,
        _variant: &str,
        _message: &str,
    ) -> Result<(), StoreError> {
        unimplemented!("Task 12")
    }

    async fn skill_kv_get(&self, _skill: &str, _key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        unimplemented!("Task 13")
    }

    async fn skill_kv_set(&self, _skill: &str, _key: &str, _value: &[u8]) -> Result<(), StoreError> {
        unimplemented!("Task 13")
    }

    async fn provision_satellite(
        &self,
        _id: SatelliteId,
        _api_key_hash: &str,
    ) -> Result<(), StoreError> {
        unimplemented!("Task 14")
    }

    async fn find_satellite(
        &self,
        _id: &SatelliteId,
    ) -> Result<Option<SatelliteRow>, StoreError> {
        unimplemented!("Task 14")
    }
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p athena-voice-storage --test sessions`
Expected: 3 tests pass. The `unimplemented!()` panics never fire because the session tests only exercise session methods.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): record/finalize/get session (impl Store for SqliteStore)"
```

---

## Task 11: `athena-voice-storage` — Event log methods

**Files:**
- Modify: `crates/athena-voice-storage/src/sqlite.rs` (replace `unimplemented!` in `append_event` + `list_events_by_session`)
- Create: `crates/athena-voice-storage/tests/events.rs`

**Interfaces:**
- Consumes: `Event` (core Task 4).
- Produces: `append_event(&Event) -> Result<()>`, `list_events_by_session(SessionId, u32) -> Result<Vec<EventRow>>` — ordered by id ASC.

- [ ] **Step 1: Write failing tests in `tests/events.rs`**

```rust
use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

fn seed_session(sid: SessionId) -> (SatelliteId, Locale) {
    (SatelliteId::new("phone-01").unwrap(), Locale::new("fr").unwrap())
}

#[tokio::test]
async fn append_and_list_in_order() {
    let s = store().await;
    let sid = SessionId::new_v4();
    let (sat, loc) = seed_session(sid);
    s.record_session(sid, sat.clone(), loc.clone()).await.unwrap();

    s.append_event(&Event::SessionStarted { session: sid, satellite: sat, locale: loc })
        .await
        .unwrap();
    s.append_event(&Event::TranscriptFinal { session: sid, text: "hello".into() })
        .await
        .unwrap();
    s.append_event(&Event::SessionEnded { session: sid, outcome: Outcome::Ok })
        .await
        .unwrap();

    let rows = s.list_events_by_session(sid, 100).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].kind, "session_started");
    assert_eq!(rows[1].kind, "transcript_final");
    assert_eq!(rows[2].kind, "session_ended");
}

#[tokio::test]
async fn list_respects_limit() {
    let s = store().await;
    let sid = SessionId::new_v4();
    for _ in 0..5 {
        s.append_event(&Event::LlmFallback { session: sid }).await.unwrap();
    }
    let rows = s.list_events_by_session(sid, 3).await.unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn list_only_returns_session_events() {
    let s = store().await;
    let a = SessionId::new_v4();
    let b = SessionId::new_v4();
    s.append_event(&Event::LlmFallback { session: a }).await.unwrap();
    s.append_event(&Event::LlmFallback { session: b }).await.unwrap();
    s.append_event(&Event::LlmFallback { session: a }).await.unwrap();

    let rows = s.list_events_by_session(a, 100).await.unwrap();
    assert_eq!(rows.len(), 2);
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-storage --test events`
Expected: tests panic on `unimplemented!("Task 11")`.

- [ ] **Step 3: Replace the two stubs in `src/sqlite.rs`**

```rust
async fn append_event(&self, event: &Event) -> Result<(), StoreError> {
    // Serialize the entire event so `kind` and the payload are self-describing.
    let value = serde_json::to_value(event)?;
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let session = value
        .get("session")
        .and_then(|v| v.as_str())
        .ok_or_else(|| StoreError::NotFound("event has no `session` field".into()))?
        .to_string();
    let payload = serde_json::to_string(&value)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO events (session, kind, payload, at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(session)
    .bind(kind)
    .bind(payload)
    .bind(now)
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn list_events_by_session(
    &self,
    session: SessionId,
    limit: u32,
) -> Result<Vec<EventRow>, StoreError> {
    let rows: Vec<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, session, kind, payload, at FROM events \
         WHERE session = ?1 ORDER BY id ASC LIMIT ?2",
    )
    .bind(session.to_string())
    .bind(i64::from(limit))
    .fetch_all(&self.pool)
    .await?;

    rows.into_iter()
        .map(|(id, sess, kind, payload, at)| {
            Ok(EventRow {
                id,
                session: sess.parse().map_err(|e: athena_voice_core::ids::IdError| {
                    StoreError::Db(sqlx::Error::Decode(Box::new(e)))
                })?,
                kind,
                payload: serde_json::from_str(&payload)?,
                at: DateTime::parse_from_rfc3339(&at)
                    .map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?
                    .with_timezone(&Utc),
            })
        })
        .collect()
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p athena-voice-storage --test events`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): append_event + list_events_by_session"
```

---

## Task 12: `athena-voice-storage` — Error log method

**Files:**
- Modify: `crates/athena-voice-storage/src/sqlite.rs` (replace `unimplemented!` in `append_error`)
- Create: `crates/athena-voice-storage/tests/errors.rs`

**Interfaces:**
- Consumes: `Stage` (core Task 4).
- Produces: `append_error(SessionId, Stage, &str, &str) -> Result<()>`.

- [ ] **Step 1: Write failing test in `tests/errors.rs`**

```rust
use athena_voice_core::event::Stage;
use athena_voice_core::ids::SessionId;
use athena_voice_storage::{SqliteStore, Store};

#[tokio::test]
async fn append_error_persists() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let sid = SessionId::new_v4();
    s.append_error(sid, Stage::Stt, "SttError::Timeout", "provider timed out after 5000ms")
        .await
        .unwrap();

    let row: (String, String, String) = sqlx::query_as(
        "SELECT stage, variant, message FROM errors WHERE session = ?1",
    )
    .bind(sid.to_string())
    .fetch_one(s.pool())
    .await
    .unwrap();

    assert_eq!(row.0, "stt");
    assert_eq!(row.1, "SttError::Timeout");
    assert_eq!(row.2, "provider timed out after 5000ms");
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-storage --test errors`
Expected: `unimplemented!("Task 12")` panic.

- [ ] **Step 3: Replace stub in `src/sqlite.rs`**

```rust
async fn append_error(
    &self,
    session: SessionId,
    stage: Stage,
    variant: &str,
    message: &str,
) -> Result<(), StoreError> {
    let stage_str = serde_json::to_value(stage)?
        .as_str()
        .unwrap()
        .to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO errors (session, stage, variant, message, at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(session.to_string())
    .bind(stage_str)
    .bind(variant)
    .bind(message)
    .bind(now)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Run test, verify pass**

Run: `cargo test -p athena-voice-storage --test errors`
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): append_error"
```

---

## Task 13: `athena-voice-storage` — Skill KV methods

**Files:**
- Modify: `crates/athena-voice-storage/src/sqlite.rs`
- Create: `crates/athena-voice-storage/tests/skill_kv.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `skill_kv_get(&str, &str) -> Result<Option<Vec<u8>>>`, `skill_kv_set(&str, &str, &[u8]) -> Result<()>`. `skill_kv_set` is upsert semantics.

- [ ] **Step 1: Write failing tests in `tests/skill_kv.rs`**

```rust
use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn set_then_get() {
    let s = store().await;
    s.skill_kv_set("weather", "last_city", b"Paris").await.unwrap();
    let v = s.skill_kv_get("weather", "last_city").await.unwrap();
    assert_eq!(v.as_deref(), Some(b"Paris".as_slice()));
}

#[tokio::test]
async fn get_missing_returns_none() {
    let s = store().await;
    let v = s.skill_kv_get("weather", "nope").await.unwrap();
    assert!(v.is_none());
}

#[tokio::test]
async fn set_upserts() {
    let s = store().await;
    s.skill_kv_set("weather", "last_city", b"Paris").await.unwrap();
    s.skill_kv_set("weather", "last_city", b"Lyon").await.unwrap();
    let v = s.skill_kv_get("weather", "last_city").await.unwrap();
    assert_eq!(v.as_deref(), Some(b"Lyon".as_slice()));
}

#[tokio::test]
async fn kvs_are_scoped_by_skill() {
    let s = store().await;
    s.skill_kv_set("weather", "k", b"A").await.unwrap();
    s.skill_kv_set("timer",   "k", b"B").await.unwrap();
    assert_eq!(s.skill_kv_get("weather", "k").await.unwrap().as_deref(), Some(b"A".as_slice()));
    assert_eq!(s.skill_kv_get("timer",   "k").await.unwrap().as_deref(), Some(b"B".as_slice()));
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-storage --test skill_kv`
Expected: `unimplemented!("Task 13")`.

- [ ] **Step 3: Replace stubs**

```rust
async fn skill_kv_get(&self, skill: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT value FROM skill_kv WHERE skill = ?1 AND key = ?2",
    )
    .bind(skill)
    .bind(key)
    .fetch_optional(&self.pool)
    .await?;
    Ok(row.map(|(v,)| v))
}

async fn skill_kv_set(&self, skill: &str, key: &str, value: &[u8]) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO skill_kv (skill, key, value) VALUES (?1, ?2, ?3) \
         ON CONFLICT(skill, key) DO UPDATE SET value = excluded.value",
    )
    .bind(skill)
    .bind(key)
    .bind(value)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p athena-voice-storage --test skill_kv`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): skill_kv_get/set (upsert)"
```

---

## Task 14: `athena-voice-storage` — Satellite methods

**Files:**
- Modify: `crates/athena-voice-storage/src/sqlite.rs`
- Create: `crates/athena-voice-storage/tests/satellites.rs`

**Interfaces:**
- Consumes: `SatelliteId`.
- Produces: `provision_satellite(SatelliteId, &str) -> Result<()>` (upsert), `find_satellite(&SatelliteId) -> Result<Option<SatelliteRow>>`.

- [ ] **Step 1: Write failing tests in `tests/satellites.rs`**

```rust
use athena_voice_core::ids::SatelliteId;
use athena_voice_storage::{SqliteStore, Store};

#[tokio::test]
async fn provision_and_find() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-01").unwrap();
    s.provision_satellite(id.clone(), "hash-abc").await.unwrap();

    let row = s.find_satellite(&id).await.unwrap().unwrap();
    assert_eq!(row.id, id);
    assert_eq!(row.api_key_hash, "hash-abc");
    assert!(row.last_seen.is_none());
}

#[tokio::test]
async fn find_missing_returns_none() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-99").unwrap();
    assert!(s.find_satellite(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn provision_upserts_key_hash() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-01").unwrap();
    s.provision_satellite(id.clone(), "hash-old").await.unwrap();
    s.provision_satellite(id.clone(), "hash-new").await.unwrap();
    let row = s.find_satellite(&id).await.unwrap().unwrap();
    assert_eq!(row.api_key_hash, "hash-new");
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p athena-voice-storage --test satellites`
Expected: `unimplemented!("Task 14")`.

- [ ] **Step 3: Replace stubs**

```rust
async fn provision_satellite(
    &self,
    id: SatelliteId,
    api_key_hash: &str,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO satellites (id, api_key_hash, created_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET api_key_hash = excluded.api_key_hash",
    )
    .bind(id.as_str())
    .bind(api_key_hash)
    .bind(now)
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn find_satellite(
    &self,
    id: &SatelliteId,
) -> Result<Option<SatelliteRow>, StoreError> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, api_key_hash, created_at, last_seen FROM satellites WHERE id = ?1",
    )
    .bind(id.as_str())
    .fetch_optional(&self.pool)
    .await?;
    let Some((raw_id, hash, created, last_seen)) = row else { return Ok(None) };
    Ok(Some(SatelliteRow {
        id: SatelliteId::new(raw_id).map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?,
        api_key_hash: hash,
        created_at: DateTime::parse_from_rfc3339(&created)
            .map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?
            .with_timezone(&Utc),
        last_seen: last_seen
            .map(|s| DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)))
            .transpose()
            .map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))?,
    }))
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p athena-voice-storage --test satellites`
Expected: 3 tests pass.

- [ ] **Step 5: Full storage test suite check**

Run: `cargo test -p athena-voice-storage`
Expected: all storage tests pass (13 across the 5 test files).

Run: `cargo clippy -p athena-voice-storage -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-storage
git commit -m "feat(storage): provision_satellite + find_satellite (upsert)"
```

---

## Task 15: `athena-voice-cli` — CLI parsing skeleton

**Files:**
- Create: `crates/athena-voice-cli/Cargo.toml`
- Create: `crates/athena-voice-cli/src/main.rs`
- Create: `crates/athena-voice-cli/src/cli.rs`

**Interfaces:**
- Consumes: nothing (bin crate).
- Produces:
  - `pub struct Cli { pub command: Command }` — derives `clap::Parser`.
  - `pub enum Command { Serve(ServeArgs) }`.
  - `pub struct ServeArgs { pub config: PathBuf, pub dry_run: bool }` (config defaults to `./athena.toml`).
  - The binary target is named `athena-voice`.

- [ ] **Step 1: Create `crates/athena-voice-cli/Cargo.toml`**

```toml
[package]
name = "athena-voice-cli"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Athena-Voice command-line binary."

[[bin]]
name = "athena-voice"
path = "src/main.rs"

[dependencies]
anyhow = { workspace = true }
clap = { workspace = true }
figment = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }

athena-voice-core = { path = "../athena-voice-core" }
athena-voice-storage = { path = "../athena-voice-storage" }

[dev-dependencies]
assert_cmd = { workspace = true }
predicates = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Write failing test in `crates/athena-voice-cli/tests/cli.rs`** (create file)

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_shows_serve_subcommand() {
    Command::cargo_bin("athena-voice")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("serve"));
}

#[test]
fn serve_help_shows_dry_run_flag() {
    Command::cargo_bin("athena-voice")
        .unwrap()
        .args(["serve", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--config"));
}
```

- [ ] **Step 3: Create `src/cli.rs`**

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "athena-voice",
    version,
    about = "Extensible voice-assistant framework.",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the Athena-Voice server.
    Serve(ServeArgs),
}

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Path to the TOML config file.
    #[arg(long, default_value = "./athena.toml", env = "ATHENA_CONFIG")]
    pub config: PathBuf,

    /// Load config + open storage, then exit without accepting traffic.
    #[arg(long)]
    pub dry_run: bool,
}
```

- [ ] **Step 4: Create `src/main.rs`**

```rust
#![deny(warnings)]

use clap::Parser;

mod cli;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::Serve(args) => {
            println!("stub: serve {args:?}");
            Ok(())
        }
    }
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-cli --test cli`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-cli
git commit -m "feat(cli): clap skeleton with serve --dry-run subcommand"
```

---

## Task 16: `athena-voice-cli` — Config loading

**Files:**
- Create: `crates/athena-voice-cli/src/config.rs`
- Modify: `crates/athena-voice-cli/src/main.rs`
- Create: `athena.example.toml` (repo root)

**Interfaces:**
- Consumes: `Locale` (core Task 2).
- Produces:
  - `pub struct Config { pub server: ServerConfig, pub storage: StorageConfig, pub locales: Vec<Locale> }`.
  - `pub struct ServerConfig { pub host: String, pub port: u16 }`.
  - `pub struct StorageConfig { pub database_url: String }` (e.g. `"sqlite:./athena.db"`).
  - `pub fn load(path: &Path) -> Result<Config, ConfigError>` — TOML + env overrides via `figment`. Env prefix `ATHENA__` with `__` as separator (`ATHENA__SERVER__PORT=9000`).
  - `pub enum ConfigError { … }`.

- [ ] **Step 1: Create `athena.example.toml` at repo root**

```toml
# Athena-Voice — example configuration.
# Copy to `athena.toml` and adjust.

locales = ["fr", "en"]

[server]
host = "0.0.0.0"
port = 8080

[storage]
# Any sqlx-compatible URL. Use "sqlite::memory:" for ephemeral tests.
database_url = "sqlite:./athena.db"
```

- [ ] **Step 2: Write failing tests in `crates/athena-voice-cli/tests/config.rs`** (create file)

```rust
use athena_voice_cli::config::{Config, load};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_config(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

#[test]
fn parses_valid_toml() {
    let f = write_config(
        r#"
locales = ["fr", "en"]

[server]
host = "127.0.0.1"
port = 9000

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    let c: Config = load(f.path()).unwrap();
    assert_eq!(c.server.host, "127.0.0.1");
    assert_eq!(c.server.port, 9000);
    assert_eq!(c.storage.database_url, "sqlite::memory:");
    assert_eq!(c.locales.len(), 2);
}

#[test]
fn env_overrides_toml() {
    let f = write_config(
        r#"
locales = ["fr"]

[server]
host = "127.0.0.1"
port = 8080

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    // figment env: ATHENA__SERVER__PORT overrides [server].port
    // Edition 2024: set_var/remove_var are unsafe. Nextest runs each test in its own
    // process, so this global mutation cannot race with other tests.
    // SAFETY: single-threaded test process; no other threads reading env.
    unsafe {
        std::env::set_var("ATHENA__SERVER__PORT", "9999");
    }
    let c: Config = load(f.path()).unwrap();
    unsafe {
        std::env::remove_var("ATHENA__SERVER__PORT");
    }
    assert_eq!(c.server.port, 9999);
}

#[test]
fn rejects_invalid_locale() {
    let f = write_config(
        r#"
locales = ["french"]

[server]
host = "0.0.0.0"
port = 8080

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    assert!(load(f.path()).is_err());
}

#[test]
fn missing_file_returns_error() {
    let path = std::path::Path::new("/definitely/not/a/real/path.toml");
    assert!(load(path).is_err());
}
```

- [ ] **Step 3: Add `tempfile` to `crates/athena-voice-cli/Cargo.toml`**

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Verify fail**

Run: `cargo test -p athena-voice-cli --test config`
Expected: compile error.

- [ ] **Step 5: Update `src/main.rs` to expose the `config` module** — replace with a lib+bin layout by promoting to a small `lib.rs`:

Create `crates/athena-voice-cli/src/lib.rs`:

```rust
#![deny(warnings)]

pub mod cli;
pub mod config;
```

Update `crates/athena-voice-cli/src/main.rs`:

```rust
#![deny(warnings)]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = athena_voice_cli::cli::Cli::parse();
    match cli.command {
        athena_voice_cli::cli::Command::Serve(args) => {
            let cfg = athena_voice_cli::config::load(&args.config)?;
            println!("stub: serve {args:?} → {cfg:?}");
            Ok(())
        }
    }
}
```

Update `crates/athena-voice-cli/Cargo.toml`:

```toml
[lib]
path = "src/lib.rs"

[[bin]]
name = "athena-voice"
path = "src/main.rs"
```

- [ ] **Step 6: Create `src/config.rs`**

```rust
use std::path::Path;

use figment::providers::{Env, Format, Toml};
use figment::{Error as FigmentError, Figment};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use athena_voice_core::ids::Locale;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub locales: Vec<Locale>,
    pub server: ServerConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub database_url: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config: {0}")]
    Parse(#[from] FigmentError),

    #[error("invalid config: {0}")]
    Invalid(String),
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    // figment doesn't verify existence itself if there's other providers, so we do:
    if !path.exists() {
        return Err(ConfigError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "config file not found"),
        });
    }
    let fig = Figment::new()
        .merge(Toml::file(path))
        .merge(Env::prefixed("ATHENA__").split("__"));

    let cfg: Config = fig.extract()?;

    if cfg.locales.is_empty() {
        return Err(ConfigError::Invalid("`locales` must not be empty".into()));
    }
    Ok(cfg)
}
```

- [ ] **Step 7: Run tests, verify pass**

Run: `cargo test -p athena-voice-cli --test config`
Expected: 4 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates athena.example.toml
git commit -m "feat(cli): figment-based TOML+env config loading"
```

---

## Task 17: `athena-voice-cli` — Tracing initialization

**Files:**
- Create: `crates/athena-voice-cli/src/logging.rs`
- Modify: `crates/athena-voice-cli/src/lib.rs` (add `pub mod logging;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn init() -> Result<(), LoggingError>` — sets up `tracing_subscriber` with JSON output on stdout, respects `RUST_LOG` env var, defaults to `info`. Idempotent (safe to call once; subsequent calls return `LoggingError::AlreadyInit`).

- [ ] **Step 1: Add to `src/lib.rs`**

```rust
pub mod logging;
```

- [ ] **Step 2: Write failing test in `crates/athena-voice-cli/tests/logging.rs`** (create file)

```rust
// A minimal smoke test: init() succeeds once, then errors.
// We can't easily capture the JSON output from an isolated global subscriber,
// so we just verify init behaviour + that calling info!/warn! doesn't panic.
use athena_voice_cli::logging;

#[test]
fn init_is_idempotent_error() {
    // First call succeeds…
    logging::init().unwrap();
    tracing::info!(target: "smoke", "hello");
    tracing::warn!(target: "smoke", value = 42, "warned");
    // …a second call must return an AlreadyInit error rather than panic.
    let err = logging::init().unwrap_err();
    assert!(matches!(err, logging::LoggingError::AlreadyInit));
}
```

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-cli --test logging`
Expected: `logging` module unresolved.

- [ ] **Step 4: Implement in `src/logging.rs`**

```rust
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

static INIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("tracing subscriber already initialised")]
    AlreadyInit,

    #[error("failed to install subscriber: {0}")]
    Install(String),
}

pub fn init() -> Result<(), LoggingError> {
    if INIT.swap(true, Ordering::SeqCst) {
        return Err(LoggingError::AlreadyInit);
    }
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = fmt::layer().json().with_current_span(true).with_span_list(true);
    tracing_subscriber::registry()
        .with(filter)
        .with(json)
        .try_init()
        .map_err(|e| LoggingError::Install(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p athena-voice-cli --test logging`
Expected: 1 test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-cli
git commit -m "feat(cli): tracing subscriber (JSON, RUST_LOG-driven, idempotent)"
```

---

## Task 18: `athena-voice-cli` — `serve --dry-run` end-to-end wiring

**Files:**
- Create: `crates/athena-voice-cli/src/serve.rs`
- Modify: `crates/athena-voice-cli/src/lib.rs` (add `pub mod serve;`)
- Modify: `crates/athena-voice-cli/src/main.rs` (delegate to `serve::run`)

**Interfaces:**
- Consumes: `Config` (Task 16), `SqliteStore` (Task 9), `logging::init` (Task 17).
- Produces:
  - `pub async fn run(args: cli::ServeArgs) -> anyhow::Result<()>` — loads config → inits logging → opens `SqliteStore` → logs `ready` → in dry-run mode, exits; otherwise, waits for SIGTERM (this Plan-1 version returns immediately even without `--dry-run` — actual event-loop wires up in Plan 2).

- [ ] **Step 1: Add to `src/lib.rs`**

```rust
pub mod serve;
```

- [ ] **Step 2: Write failing integration test in `crates/athena-voice-cli/tests/serve_dry_run.rs`** (create file)

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn dry_run_exits_zero_and_logs_ready() {
    // A minimal config using in-memory SQLite so we don't touch the filesystem.
    let mut cfg = NamedTempFile::new().unwrap();
    writeln!(
        cfg,
        r#"
locales = ["fr", "en"]

[server]
host = "127.0.0.1"
port = 0

[storage]
database_url = "sqlite::memory:"
        "#
    )
    .unwrap();

    Command::cargo_bin("athena-voice")
        .unwrap()
        .args(["serve", "--dry-run", "--config", cfg.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"ready\"").or(predicate::str::contains("ready")));
}
```

Note: JSON tracing writes to stdout by default. If the test relies on stderr, adjust the JSON layer. To keep the test resilient, we check both — `assert_cmd`'s `stderr` matcher will only match if the output really is there; alternative: check the process succeeded and skip the log-content assertion. We use both `stderr` and `stdout` predicates in practice: switch the assertion to `.stdout(predicate::str::contains(...))` if `tracing`'s default writer is stdout.

- [ ] **Step 3: Verify fail**

Run: `cargo test -p athena-voice-cli --test serve_dry_run`
Expected: `serve` module unresolved.

- [ ] **Step 4: Implement `src/serve.rs`**

```rust
use athena_voice_storage::SqliteStore;

use crate::{cli::ServeArgs, config, logging};

pub async fn run(args: ServeArgs) -> anyhow::Result<()> {
    let cfg = config::load(&args.config)?;

    // Logging: ignore AlreadyInit so nested invocations in tests don't fail.
    match logging::init() {
        Ok(()) | Err(logging::LoggingError::AlreadyInit) => {}
        Err(e) => anyhow::bail!("logging init failed: {e}"),
    }

    let _store = SqliteStore::open(&cfg.storage.database_url).await?;

    tracing::info!(
        host = %cfg.server.host,
        port = cfg.server.port,
        locales = ?cfg.locales.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
        "ready"
    );

    if args.dry_run {
        tracing::info!("dry-run: exiting");
        return Ok(());
    }

    // Plan 2 wires the actor DAG here. For now, wait once for Ctrl-C then exit.
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    Ok(())
}
```

- [ ] **Step 5: Update `src/main.rs`**

```rust
#![deny(warnings)]

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = athena_voice_cli::cli::Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        match cli.command {
            athena_voice_cli::cli::Command::Serve(args) => {
                athena_voice_cli::serve::run(args).await
            }
        }
    })
}
```

- [ ] **Step 6: Route tracing output to stdout for the test** — the default `fmt::layer().json()` writes to stdout. Test asserts against `.stdout(...)`. Update the assertion:

```rust
.assert()
.success()
.stdout(predicate::str::contains("ready"));
```

- [ ] **Step 7: Run tests, verify pass**

Run: `cargo test -p athena-voice-cli --test serve_dry_run -- --nocapture`
Expected: test passes (may take a few seconds for the first `cargo` run).

- [ ] **Step 8: Sanity-check the full workspace**

Run: `cargo build --workspace`
Expected: clean build.

Run: `cargo test --workspace`
Expected: all tests pass across `athena-voice-core` (~15), `athena-voice-storage` (~13), `athena-voice-cli` (~9).

Run: `cargo clippy --workspace --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/athena-voice-cli
git commit -m "feat(cli): serve --dry-run wires config + storage + logging end-to-end"
```

---

## Task 19: Nextest + cargo-deny configuration

**Files:**
- Create: `.cargo/nextest.toml`
- Create: `deny.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: reusable test-runner + supply-chain-audit configuration.

- [ ] **Step 1: Create `.cargo/nextest.toml`**

```toml
[profile.default]
retries = 0
fail-fast = false
slow-timeout = { period = "30s", terminate-after = 3 }

[profile.ci]
retries = { backoff = "fixed", count = 1, delay = "2s" }
fail-fast = false
slow-timeout = { period = "60s", terminate-after = 3 }
```

- [ ] **Step 2: Create `deny.toml`**

```toml
[graph]
targets = [
    { triple = "x86_64-unknown-linux-gnu" },
    { triple = "aarch64-unknown-linux-gnu" },
]

[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
version = 2
yanked = "deny"

[licenses]
version = 2
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
    "Unicode-3.0",
    "Zlib",
    "CC0-1.0",
    "MPL-2.0",
]
confidence-threshold = 0.9

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 3: Verify locally**

Run: `cargo install cargo-nextest --locked` (only if not installed)
Run: `cargo nextest run --workspace`
Expected: all tests pass (same as `cargo test`).

Run: `cargo install cargo-deny --locked` (only if not installed)
Run: `cargo deny check`
Expected: no advisories, no license violations, no unknown sources.

- [ ] **Step 4: Commit**

```bash
git add .cargo/nextest.toml deny.toml
git commit -m "chore: nextest profile + cargo-deny policy"
```

---

## Task 20: CI workflow (`.github/workflows/ci.yml`)

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: everything above.
- Produces: a green CI run on `linux-amd64` covering fmt + clippy + deny + nextest + llvm-cov.

- [ ] **Step 1: Create `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  pull_request:
  push:
    branches: [main]

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  fmt:
    name: rustfmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  clippy:
    name: clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-features --all-targets -- -D warnings

  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with:
          command: check

  test:
    name: nextest
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with: { tool: nextest }
      - run: cargo nextest run --workspace --profile ci

  coverage:
    name: llvm-cov
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with: { tool: cargo-llvm-cov,nextest }
      - name: Generate coverage
        run: cargo llvm-cov nextest --workspace --lcov --output-path lcov.info
      - uses: codecov/codecov-action@v4
        with:
          files: lcov.info
          fail_ci_if_error: false
```

- [ ] **Step 2: Verify locally with `act` (optional but recommended)**

If you have `act` installed:
Run: `act pull_request -j fmt`
Expected: passes.

Otherwise trust and push — the next commit push to your branch will trigger CI.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: fmt + clippy + deny + nextest + llvm-cov"
```

- [ ] **Step 4: Push and observe first CI run**

```bash
git push -u origin HEAD
```

Watch the Actions tab on GitHub. Expected: all five jobs green.

If a job fails, investigate root cause (do NOT skip / disable the failing job). Common first-run failures:
- `Cargo.lock` not committed → add & commit it.
- Warnings surfaced as errors in clippy → fix the code, don't relax the lint.
- `cargo-deny` license unknowns → adjust `deny.toml` allowlist only after confirming the license is truly acceptable.

---

## Definition of Done for Plan 1

Plan 1 is complete when all of these are true:

1. `cargo build --workspace` succeeds on a clean checkout with only the toolchain installed.
2. `cargo nextest run --workspace` — all tests green (target: ~40 tests across the three crates).
3. `cargo clippy --workspace --all-features --all-targets -- -D warnings` — no warnings.
4. `cargo fmt --all --check` — no diff.
5. `cargo deny check` — no advisories / license violations.
6. `athena-voice serve --dry-run --config athena.example.toml` runs, logs one JSON `"ready"` line + one `"dry-run: exiting"` line, exits 0.
7. GitHub Actions CI green on the branch (`fmt`, `clippy`, `deny`, `test`, `coverage` jobs all pass).
8. `docs/superpowers/plans/2026-07-10-athena-voice-foundation.md` (this file) exists in the tree.
9. Ready for Plan 2 (voice pipeline with fakes) to depend on: `athena-voice-core` types + traits, `athena-voice-storage` `Store` trait + `SqliteStore` impl, `athena-voice-cli::config::Config` schema (Plan 2 will extend it with mqtt + provider sections).

## What Plan 1 explicitly does NOT deliver (intentional)

- No MQTT client, no `SatelliteAdapter`.
- No pipeline actors.
- No provider implementations (traits only; concrete impls come in Plans 2–3).
- No WASM host (Plan 4).
- No dashboard (Plan 5).
- No Docker Compose file (Plan 5).
- No local ML provider bindings (`whisper-rs` / `llama-cpp-rs` / `piper-rs` — Plan 3 or v1.1).
- No arm64 CI (added in Plan 5's release workflow).
- No integration tests spanning multiple crates (they arrive with the pipeline, Plan 2).
- No E2E `testcontainers` tests (Plan 3).

These are all deliberate. Do not sneak them into Plan 1 tasks.
