# Web Configuration Interface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A token-protected web UI (embedded in the `serve` binary) to configure, enable/disable, and install WASM skills, with all web-edited config — including secrets — stored in SQLite instead of tracked TOML files.

**Architecture:** New crate `athena-voice-admin` hosts an axum router spawned by `serve` alongside the MQTT runtime, binding the previously unused `[server] host/port`. It reads/writes new SQLite tables via `athena-voice-storage`, and applies skill changes live through a `SkillsHandle` (registry + deps) newly exposed by `athena-voice-runtime`. Skills declare a config schema via a new optional `config_schema` guest export; the UI renders forms from it. Spec: `docs/superpowers/specs/2026-07-24-web-config-design.md`.

**Tech Stack:** Rust edition 2024 (rust 1.95), axum 0.8, argon2 0.5, include_dir 0.7, sqlx/SQLite, extism 1, vanilla JS frontend (no npm).

## Global Constraints

- Every commit uses the Gekkotron identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. Never commit with any other name.
- Commit messages are plain imperative sentences (match `git log`: "Jeedom sensor connector skill"), no `feat:`/`fix:` prefixes.
- New crates start with `#![deny(warnings)]` (workspace convention) and inherit `[workspace.lints]` via `[lints] workspace = true`.
- Workspace clippy runs pedantic: run `cargo clippy -p <crate> --all-targets` before each commit; it must be warning-free.
- Secret values must never appear in any tracked file, any API response, or any log line. API responses represent secrets as `{"set": true|false}`.
- axum 0.8 path-parameter syntax is `{name}` (not `:name`).
- The four bundled skill `.wasm` binaries are committed under `skills/`; rebuilding them requires `rustup target add wasm32-wasip1` and each crate's `./build.sh`.
- Tests that need a store use `SqliteStore::open("sqlite::memory:")` — no filesystem DB in tests.

---

### Task 1: Secrets remediation (Part 0 of the spec)

The Jeedom API key `JJ5qGwlquxyayFlfqYc5COM9Ee5IQZJpQD3T0O8V6yEwZ9dMAvYSu4JetQJPLn4b` is committed and pushed to the public repo. The user rotates it in Jeedom (out of scope for you); this task removes every tracked occurrence and guards against recurrence. No history rewrite (decided in spec).

**Files:**
- Modify: `athena.voice.toml` (the `[skills.jeedom]` section, currently lines 51–55)
- Modify: `.gitignore`
- Possibly modify: any other tracked file the grep in Step 1 finds

**Interfaces:**
- Produces: a repo where `git grep JJ5qGwlquxyayFlfqYc5` returns nothing; later tasks assume the Jeedom section in `athena.voice.toml` is a comment-only placeholder.

- [ ] **Step 1: Find every tracked occurrence of the key**

Run: `git grep -l "JJ5qGwlquxyayFlfqYc5"`
Expected: `athena.voice.toml` (possibly others — Step 2 applies to all hits).

- [ ] **Step 2: Replace the Jeedom section in `athena.voice.toml`**

Replace the entire `[skills.jeedom]` block (comment included) with:

```toml
# Jeedom sensors — configure via the web UI (http://127.0.0.1:8080 once
# `serve` is running): the API key and sensor list are stored in the
# SQLite database, never in this file. See README "Web configuration".
# Leaving this section absent is fine; the skill stays dormant until
# configured.
```

Apply the same treatment to every other file Step 1 found (keep each file's surrounding content intact, remove only the secret-bearing lines). The working tree already has an uncommitted IP change in this file — this replacement supersedes it.

- [ ] **Step 3: Add gitignore guards**

Append to `.gitignore`:

```gitignore
# Local secrets and databases must never be committed
*.secrets.toml
*.db
*.db-shm
*.db-wal
```

- [ ] **Step 4: Verify**

Run: `git grep "JJ5qGwlquxyayFlfqYc5" || echo CLEAN`
Expected: `CLEAN`
Run: `cargo run -p athena-voice-cli -- serve --config athena.voice.toml --dry-run`
Expected: exits 0 (config still parses without the section).

- [ ] **Step 5: Commit**

```bash
git add athena.voice.toml .gitignore
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Strip committed Jeedom API key; ignore local secrets and databases"
```

---

### Task 2: Config schema types in the skill SDK

**Files:**
- Create: `crates/athena-voice-skill-sdk/src/schema.rs`
- Modify: `crates/athena-voice-skill-sdk/src/lib.rs`

**Interfaces:**
- Produces: `athena_voice_skill_sdk::{ConfigSchema, ConfigField, FieldKind, ItemField}` — serde-serializable, shared by host (registry, admin API) and guests (skills). Exact shapes below; later tasks depend on these field names.

- [ ] **Step 1: Write the failing test**

Create `crates/athena-voice-skill-sdk/src/schema.rs` with the tests first (module body comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_jeedom_style_schema() {
        let json = r#"{ "fields": [
            { "key": "base_url", "label": "Jeedom URL", "type": "url", "required": true },
            { "key": "api_key", "label": "API key", "type": "secret", "required": true },
            { "key": "sensors", "label": "Sensors", "type": "list",
              "item_fields": [
                { "key": "name", "type": "string" },
                { "key": "id",   "type": "number" },
                { "key": "unit", "type": "string" } ] }
        ] }"#;
        let schema: ConfigSchema = serde_json::from_str(json).unwrap();
        assert_eq!(schema.fields.len(), 3);
        assert_eq!(schema.fields[0].kind, FieldKind::Url);
        assert!(schema.fields[1].is_secret());
        assert!(!schema.fields[0].is_secret());
        assert_eq!(schema.fields[2].item_fields[1].kind, FieldKind::Number);
        // Optional fields default cleanly.
        assert!(!schema.fields[2].required);
        assert!(schema.fields[0].help.is_empty());
        let back = serde_json::to_string(&schema).unwrap();
        let again: ConfigSchema = serde_json::from_str(&back).unwrap();
        assert_eq!(schema, again);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p athena-voice-skill-sdk schema -- --nocapture`
Expected: compile FAIL (`ConfigSchema` not defined).

- [ ] **Step 3: Implement the types**

Prepend to `crates/athena-voice-skill-sdk/src/schema.rs`:

```rust
//! Skill config schema — the optional `config_schema` guest export returns
//! this as JSON so the admin UI can render a typed form. Skills without the
//! export get a free-form key/value editor instead.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSchema {
    pub fields: Vec<ConfigField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub default: String,
    /// Item shape for `FieldKind::List` fields; empty otherwise.
    #[serde(default)]
    pub item_fields: Vec<ItemField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    String,
    Number,
    Secret,
    Url,
    Host,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemField {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
}

impl ConfigField {
    pub fn is_secret(&self) -> bool {
        matches!(self.kind, FieldKind::Secret)
    }
}
```

In `crates/athena-voice-skill-sdk/src/lib.rs` add after `pub mod skill;`:

```rust
pub mod schema;
```

and extend the re-exports:

```rust
pub use schema::{ConfigField, ConfigSchema, FieldKind, ItemField};
```

Note: the SDK already depends on `serde` and `serde_json`? Check `crates/athena-voice-skill-sdk/Cargo.toml` — if `serde`/`serde_json` are missing, add `serde = { workspace = true }` and (dev-)dependency `serde_json = { workspace = true }`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p athena-voice-skill-sdk schema`
Expected: PASS. Also run `cargo clippy -p athena-voice-skill-sdk --all-targets` — no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-skill-sdk
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Skill SDK: config schema types for admin UI forms"
```

---

### Task 3: Storage — admin tables and Store methods

**Files:**
- Create: `crates/athena-voice-storage/migrations/0004_admin.sql`
- Modify: `crates/athena-voice-storage/src/models.rs`
- Modify: `crates/athena-voice-storage/src/store.rs`
- Modify: `crates/athena-voice-storage/src/sqlite.rs`
- Modify: `crates/athena-voice-storage/src/lib.rs` (InMemoryStore)

**Interfaces:**
- Produces (on `trait Store`, all `async`, all returning `Result<_, StoreError>`):
  - `skill_settings_for(&self, skill: &str) -> Vec<SkillSettingRow>`
  - `skill_settings_all(&self) -> Vec<SkillSettingRow>`
  - `skill_setting_set(&self, skill: &str, key: &str, value: &str, is_secret: bool) -> ()`
  - `skill_setting_delete(&self, skill: &str, key: &str) -> bool`
  - `skill_enabled_set(&self, skill: &str, enabled: bool) -> ()`
  - `skills_disabled(&self) -> Vec<String>`
  - `admin_token_hash(&self) -> Option<String>`
  - `admin_token_hash_set(&self, hash: &str) -> ()`
- Produces: `athena_voice_storage::models::SkillSettingRow { skill: String, key: String, value: String, is_secret: bool }`

- [ ] **Step 1: Write the migration**

Create `crates/athena-voice-storage/migrations/0004_admin.sql`:

```sql
-- Admin/web-UI state: per-skill settings (secrets included — this file
-- lives outside the repo), enable flags, and the admin token hash.
CREATE TABLE IF NOT EXISTS skill_settings (
    skill      TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    is_secret  INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    PRIMARY KEY (skill, key)
);

CREATE TABLE IF NOT EXISTS skill_state (
    skill   TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS admin_auth (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    token_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);
```

- [ ] **Step 2: Write the failing tests**

Append to `crates/athena-voice-storage/src/sqlite.rs` (inside its existing `#[cfg(test)] mod tests` if present, otherwise create one at the bottom):

```rust
#[tokio::test]
async fn skill_settings_roundtrip_and_upsert() {
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    store.skill_setting_set("jeedom", "api_key", "abc", true).await.unwrap();
    store.skill_setting_set("jeedom", "base_url", "http://x", false).await.unwrap();
    store.skill_setting_set("jeedom", "api_key", "def", true).await.unwrap(); // upsert

    let rows = store.skill_settings_for("jeedom").await.unwrap();
    assert_eq!(rows.len(), 2);
    let key_row = rows.iter().find(|r| r.key == "api_key").unwrap();
    assert_eq!(key_row.value, "def");
    assert!(key_row.is_secret);

    let all = store.skill_settings_all().await.unwrap();
    assert_eq!(all.len(), 2);

    assert!(store.skill_setting_delete("jeedom", "api_key").await.unwrap());
    assert!(!store.skill_setting_delete("jeedom", "api_key").await.unwrap());
    assert_eq!(store.skill_settings_for("jeedom").await.unwrap().len(), 1);
}

#[tokio::test]
async fn skill_enabled_flag_and_disabled_list() {
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    assert!(store.skills_disabled().await.unwrap().is_empty());
    store.skill_enabled_set("timer", false).await.unwrap();
    store.skill_enabled_set("weather", true).await.unwrap();
    assert_eq!(store.skills_disabled().await.unwrap(), vec!["timer".to_string()]);
    store.skill_enabled_set("timer", true).await.unwrap(); // upsert back on
    assert!(store.skills_disabled().await.unwrap().is_empty());
}

#[tokio::test]
async fn admin_token_hash_single_row() {
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();
    assert!(store.admin_token_hash().await.unwrap().is_none());
    store.admin_token_hash_set("h1").await.unwrap();
    assert_eq!(store.admin_token_hash().await.unwrap().as_deref(), Some("h1"));
    store.admin_token_hash_set("h2").await.unwrap(); // replace
    assert_eq!(store.admin_token_hash().await.unwrap().as_deref(), Some("h2"));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p athena-voice-storage skill_settings admin_token skill_enabled`
Expected: compile FAIL (methods not on `Store`).

- [ ] **Step 4: Implement**

`crates/athena-voice-storage/src/models.rs` — append:

```rust
/// One web-edited skill setting; overrides the same key from TOML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSettingRow {
    pub skill: String,
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}
```

`crates/athena-voice-storage/src/store.rs` — add to the `Store` trait (and `use crate::models::SkillSettingRow;`):

```rust
    async fn skill_settings_for(&self, skill: &str) -> Result<Vec<SkillSettingRow>, StoreError>;

    async fn skill_settings_all(&self) -> Result<Vec<SkillSettingRow>, StoreError>;

    async fn skill_setting_set(
        &self,
        skill: &str,
        key: &str,
        value: &str,
        is_secret: bool,
    ) -> Result<(), StoreError>;

    async fn skill_setting_delete(&self, skill: &str, key: &str) -> Result<bool, StoreError>;

    async fn skill_enabled_set(&self, skill: &str, enabled: bool) -> Result<(), StoreError>;

    async fn skills_disabled(&self) -> Result<Vec<String>, StoreError>;

    async fn admin_token_hash(&self) -> Result<Option<String>, StoreError>;

    async fn admin_token_hash_set(&self, hash: &str) -> Result<(), StoreError>;
```

`crates/athena-voice-storage/src/sqlite.rs` — add to `impl Store for SqliteStore` (follow the file's existing query style; `sqlx::query`/`query_as` with `?` binds):

```rust
    async fn skill_settings_for(&self, skill: &str) -> Result<Vec<SkillSettingRow>, StoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT skill, key, value, is_secret FROM skill_settings WHERE skill = ? ORDER BY key",
        )
        .bind(skill)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(skill, key, value, is_secret)| SkillSettingRow { skill, key, value, is_secret: is_secret != 0 })
            .collect())
    }

    async fn skill_settings_all(&self) -> Result<Vec<SkillSettingRow>, StoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT skill, key, value, is_secret FROM skill_settings ORDER BY skill, key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(skill, key, value, is_secret)| SkillSettingRow { skill, key, value, is_secret: is_secret != 0 })
            .collect())
    }

    async fn skill_setting_set(
        &self,
        skill: &str,
        key: &str,
        value: &str,
        is_secret: bool,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO skill_settings (skill, key, value, is_secret, updated_at)
             VALUES (?, ?, ?, ?, strftime('%s','now'))
             ON CONFLICT (skill, key) DO UPDATE
             SET value = excluded.value, is_secret = excluded.is_secret,
                 updated_at = excluded.updated_at",
        )
        .bind(skill)
        .bind(key)
        .bind(value)
        .bind(i64::from(is_secret))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn skill_setting_delete(&self, skill: &str, key: &str) -> Result<bool, StoreError> {
        let res = sqlx::query("DELETE FROM skill_settings WHERE skill = ? AND key = ?")
            .bind(skill)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn skill_enabled_set(&self, skill: &str, enabled: bool) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO skill_state (skill, enabled) VALUES (?, ?)
             ON CONFLICT (skill) DO UPDATE SET enabled = excluded.enabled",
        )
        .bind(skill)
        .bind(i64::from(enabled))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn skills_disabled(&self) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT skill FROM skill_state WHERE enabled = 0 ORDER BY skill",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(s,)| s).collect())
    }

    async fn admin_token_hash(&self) -> Result<Option<String>, StoreError> {
        let row = sqlx::query_as::<_, (String,)>("SELECT token_hash FROM admin_auth WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(h,)| h))
    }

    async fn admin_token_hash_set(&self, hash: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO admin_auth (id, token_hash) VALUES (1, ?)
             ON CONFLICT (id) DO UPDATE SET token_hash = excluded.token_hash",
        )
        .bind(hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

Adjust `&self.pool` to the actual field/accessor used elsewhere in the file (there is a `pub fn pool(&self)` at `sqlite.rs:47`; the private field it returns is what other queries use).

`crates/athena-voice-storage/src/lib.rs` — give `InMemoryStore` a real implementation (admin tests may use it):

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use crate::models::SkillSettingRow;
```

Extend the struct and `Default`:

```rust
pub struct InMemoryStore {
    tmp_store: MemoryTmpStore,
    skill_settings: Mutex<HashMap<(String, String), SkillSettingRow>>,
    skill_enabled: Mutex<HashMap<String, bool>>,
    admin_token: Mutex<Option<String>>,
}
```

(update `Default::default()` to init the three new fields with `Mutex::new(...)` of empty values; `#[derive(Debug)]` still works — `Mutex` and the row type are `Debug`).

Add to `impl Store for InMemoryStore`:

```rust
    async fn skill_settings_for(&self, skill: &str) -> Result<Vec<SkillSettingRow>, StoreError> {
        let map = self.skill_settings.lock().expect("skill_settings poisoned");
        let mut rows: Vec<_> = map.values().filter(|r| r.skill == skill).cloned().collect();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(rows)
    }

    async fn skill_settings_all(&self) -> Result<Vec<SkillSettingRow>, StoreError> {
        let map = self.skill_settings.lock().expect("skill_settings poisoned");
        let mut rows: Vec<_> = map.values().cloned().collect();
        rows.sort_by(|a, b| (&a.skill, &a.key).cmp(&(&b.skill, &b.key)));
        Ok(rows)
    }

    async fn skill_setting_set(
        &self,
        skill: &str,
        key: &str,
        value: &str,
        is_secret: bool,
    ) -> Result<(), StoreError> {
        self.skill_settings.lock().expect("skill_settings poisoned").insert(
            (skill.to_string(), key.to_string()),
            SkillSettingRow {
                skill: skill.to_string(),
                key: key.to_string(),
                value: value.to_string(),
                is_secret,
            },
        );
        Ok(())
    }

    async fn skill_setting_delete(&self, skill: &str, key: &str) -> Result<bool, StoreError> {
        Ok(self
            .skill_settings
            .lock()
            .expect("skill_settings poisoned")
            .remove(&(skill.to_string(), key.to_string()))
            .is_some())
    }

    async fn skill_enabled_set(&self, skill: &str, enabled: bool) -> Result<(), StoreError> {
        self.skill_enabled
            .lock()
            .expect("skill_enabled poisoned")
            .insert(skill.to_string(), enabled);
        Ok(())
    }

    async fn skills_disabled(&self) -> Result<Vec<String>, StoreError> {
        let map = self.skill_enabled.lock().expect("skill_enabled poisoned");
        let mut out: Vec<String> = map
            .iter()
            .filter(|(_, &on)| !on)
            .map(|(k, _)| k.clone())
            .collect();
        out.sort();
        Ok(out)
    }

    async fn admin_token_hash(&self) -> Result<Option<String>, StoreError> {
        Ok(self.admin_token.lock().expect("admin_token poisoned").clone())
    }

    async fn admin_token_hash_set(&self, hash: &str) -> Result<(), StoreError> {
        *self.admin_token.lock().expect("admin_token poisoned") = Some(hash.to_string());
        Ok(())
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p athena-voice-storage`
Expected: all PASS (new and pre-existing). Then `cargo clippy -p athena-voice-storage --all-targets` — clean. Then `cargo build --workspace` — the runtime/cli mock stores, if any implement `Store` directly, will fail to compile until given the new methods; fix any such impl the same way as `InMemoryStore` (search: `grep -rn "impl Store for" crates`).

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-storage crates
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Storage: skill settings, enable flags, and admin token tables"
```

---

### Task 4: Runtime — schema retrieval and cache in the registry

**Files:**
- Modify: `crates/athena-voice-runtime/src/wasm/registry.rs`

**Interfaces:**
- Consumes: `athena_voice_skill_sdk::ConfigSchema` (Task 2).
- Produces:
  - `SkillPlugin::config_schema(&mut self) -> Option<ConfigSchema>` (trait method, default `None`)
  - `SkillRegistry::config_schema(&self, name: &str) -> Option<ConfigSchema>` (cached at install time)

- [ ] **Step 1: Write the failing test**

The registry tests already use mock `SkillPlugin`s (see the test module at the bottom of `registry.rs` — reuse its existing mock type, adding the new method). Append a test:

```rust
    #[test]
    fn install_caches_config_schema() {
        // Extend the existing mock plugin type with:
        //   fn config_schema(&mut self) -> Option<ConfigSchema> { self.schema.clone() }
        // where `schema: Option<ConfigSchema>` is a new field (None in other tests).
        let schema = ConfigSchema {
            fields: vec![ConfigField {
                key: "api_key".into(),
                label: "API key".into(),
                kind: FieldKind::Secret,
                required: true,
                help: String::new(),
                default: String::new(),
                item_fields: vec![],
            }],
        };
        let registry = SkillRegistry::new();
        let plugin = mock_plugin_with_schema(Some(schema.clone())); // helper mirroring existing mock constructors
        registry.install("jeedom", plugin, &["fr".into()]).unwrap();
        assert_eq!(registry.config_schema("jeedom"), Some(schema));
        assert_eq!(registry.config_schema("nope"), None);
        registry.remove("jeedom");
        assert_eq!(registry.config_schema("jeedom"), None);
    }
```

Adapt the mock-construction call to the actual mock in that file (read the existing tests first; the mock already implements `pattern_rules`/`handle`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p athena-voice-runtime install_caches_config_schema`
Expected: compile FAIL (`config_schema` not defined).

- [ ] **Step 3: Implement**

In `registry.rs`:

1. Import: `use athena_voice_skill_sdk::ConfigSchema;`
2. Add to `trait SkillPlugin` (after `handle`):

```rust
    /// Parsed `config_schema` guest export, if the skill provides one.
    fn config_schema(&mut self) -> Option<ConfigSchema> {
        None
    }
```

3. Implement on `ExtismSkillPlugin` (inside its `impl SkillPlugin for ...`; the plugin field name is visible in the existing `pattern_rules` impl — reuse the same call style):

```rust
    fn config_schema(&mut self) -> Option<ConfigSchema> {
        if !self.plugin.function_exists("config_schema") {
            return None;
        }
        let out = self.plugin.call::<&str, String>("config_schema", "").ok()?;
        match serde_json::from_str(&out) {
            Ok(schema) => Some(schema),
            Err(e) => {
                warn!(error = %e, "config_schema export returned invalid JSON; ignoring");
                None
            }
        }
    }
```

4. Add a cache field to `SkillRegistry` (init empty in `new()`):

```rust
    schemas: RwLock<HashMap<String, ConfigSchema>>,
```

5. In `install()`, after the `pattern_rules` loop (while you still have `plugin` before it is moved into the map — query it in its own lock scope):

```rust
        let schema = {
            let mut guard = plugin
                .lock()
                .expect("skill plugin mutex poisoned during install");
            guard.config_schema()
        };
```

and after the plugin-map insert:

```rust
        {
            let mut schemas = self.schemas.write().expect("schemas lock poisoned");
            match schema {
                Some(s) => {
                    schemas.insert(name.to_string(), s);
                }
                None => {
                    schemas.remove(name);
                }
            }
        }
```

6. In `remove()`, alongside the `plugin_rules` removal:

```rust
        self.schemas.write().expect("schemas lock poisoned").remove(name);
```

7. Getter:

```rust
    /// Config schema cached at install time; `None` for skills without the
    /// export (the UI falls back to a key/value editor).
    pub fn config_schema(&self, name: &str) -> Option<ConfigSchema> {
        self.schemas
            .read()
            .expect("schemas lock poisoned")
            .get(name)
            .cloned()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-runtime` and `cargo clippy -p athena-voice-runtime --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-runtime
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Registry: read and cache skill config schemas at install time"
```

---

### Task 5: Runtime — DB-over-TOML settings merge

**Files:**
- Create: `crates/athena-voice-runtime/src/wasm/settings.rs`
- Modify: `crates/athena-voice-runtime/src/wasm/mod.rs` (add `pub mod settings;`)

**Interfaces:**
- Consumes: `SkillConfig` (registry).
- Produces:
  - `athena_voice_runtime::wasm::settings::HTTP_ALLOWLIST_KEY: &str = "$http_allowlist"`
  - `athena_voice_runtime::wasm::settings::apply_settings(base: &SkillConfig, rows: &[(String, String)]) -> SkillConfig`
  - Rows are `(key, value)` pairs from `skill_settings`; the reserved key `$http_allowlist` (JSON array of hosts) replaces `http_allowlist`, every other key overrides `config[key]`.

- [ ] **Step 1: Write the failing test**

Create `crates/athena-voice-runtime/src/wasm/settings.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::registry::SkillConfig;
    use std::collections::HashMap;

    #[test]
    fn db_rows_override_toml_key_by_key() {
        let base = SkillConfig {
            http_allowlist: vec!["old.example".into()],
            mqtt_publish_allowlist: vec!["home/+/light/set".into()],
            config: HashMap::from([
                ("base_url".into(), "http://toml".into()),
                ("kept".into(), "from-toml".into()),
            ]),
            retention_gc_after_sec: Some(60),
            config_file: None,
        };
        let rows = vec![
            ("base_url".to_string(), "http://db".to_string()),
            ("api_key".to_string(), "s3cret".to_string()),
            (HTTP_ALLOWLIST_KEY.to_string(), r#"["192.168.1.91"]"#.to_string()),
        ];
        let merged = apply_settings(&base, &rows);
        assert_eq!(merged.config["base_url"], "http://db");     // overridden
        assert_eq!(merged.config["kept"], "from-toml");         // preserved
        assert_eq!(merged.config["api_key"], "s3cret");         // added
        assert_eq!(merged.http_allowlist, vec!["192.168.1.91"]); // replaced
        assert_eq!(merged.mqtt_publish_allowlist, base.mqtt_publish_allowlist);
        assert_eq!(merged.retention_gc_after_sec, Some(60));
        assert!(!merged.config.contains_key(HTTP_ALLOWLIST_KEY)); // reserved key not leaked
    }

    #[test]
    fn invalid_allowlist_json_keeps_base_allowlist() {
        let base = SkillConfig::default();
        let rows = vec![(HTTP_ALLOWLIST_KEY.to_string(), "not-json".to_string())];
        let merged = apply_settings(&base, &rows);
        assert!(merged.http_allowlist.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p athena-voice-runtime settings`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

Prepend to `settings.rs`:

```rust
//! Merge web-edited skill settings (SQLite rows) over the TOML-derived
//! [`SkillConfig`] base — the DB wins key-by-key, so tracked TOML never
//! needs to hold secrets.

use crate::wasm::registry::SkillConfig;

/// Reserved settings key holding a JSON array of allowed HTTP hosts; the
/// admin API derives it from schema `host`/`url` fields on config save.
pub const HTTP_ALLOWLIST_KEY: &str = "$http_allowlist";

pub fn apply_settings(base: &SkillConfig, rows: &[(String, String)]) -> SkillConfig {
    let mut out = base.clone();
    for (key, value) in rows {
        if key == HTTP_ALLOWLIST_KEY {
            match serde_json::from_str::<Vec<String>>(value) {
                Ok(hosts) => out.http_allowlist = hosts,
                Err(e) => tracing::warn!(error = %e, "invalid $http_allowlist row ignored"),
            }
        } else {
            out.config.insert(key.clone(), value.clone());
        }
    }
    out
}
```

Add `pub mod settings;` to `crates/athena-voice-runtime/src/wasm/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-runtime settings` then clippy.
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-runtime
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Runtime: merge web-edited skill settings over TOML config"
```

---

### Task 6: Runtime — expose a SkillsHandle and honor disabled skills

**Files:**
- Modify: `crates/athena-voice-runtime/src/lib.rs`

**Interfaces:**
- Consumes: `SkillRegistry`, `SkillDeps` (already `Clone`).
- Produces:
  - `SkillsInit` gains `pub disabled: Vec<String>` (skills removed right after `load_dir`).
  - `pub struct SkillsHandle { pub registry: Arc<SkillRegistry>, pub deps: SkillDeps, pub dir: PathBuf }` (`#[derive(Clone)]`).
  - `Runtime` gains `pub skills: Option<SkillsHandle>`.

- [ ] **Step 1: Add the types and wire them (no isolated unit test — `Runtime::spawn` needs a broker; compile + existing tests cover this, and Task 11's integration test exercises the handle)**

In `crates/athena-voice-runtime/src/lib.rs`:

```rust
/// Live handle to the skill layer for the admin API: reload, remove,
/// schema/name queries. `deps.per_skill` is the merged config snapshot from
/// startup; the admin API overrides entries before each reload.
#[derive(Clone)]
pub struct SkillsHandle {
    pub registry: Arc<SkillRegistry>,
    pub deps: SkillDeps,
    pub dir: PathBuf,
}
```

`SkillsInit` — add field:

```rust
    /// Skills present in `dir` but disabled via the web UI; they are
    /// unloaded right after the directory scan.
    pub disabled: Vec<String>,
```

In `Runtime::spawn`, change the skills arm to keep handles (the `Some(init)` branch):

```rust
            Some(init) => {
                let disabled = init.disabled;
                let dir = init.dir.clone();
                let deps = SkillDeps { /* unchanged construction */ };
                let registry = Arc::new(
                    SkillRegistry::load_dir(&dir, &deps)
                        .map_err(|e| RuntimeError::Config(format!("skill load: {e}")))?,
                );
                for name in &disabled {
                    if registry.remove(name) {
                        tracing::info!(skill = %name, "skill disabled via settings; unloaded");
                    }
                }
                tracing::info!(
                    dir = %dir.display(),
                    skills = ?registry.skill_names(),
                    "skills loaded"
                );
                let rules = registry.patterns_handle();
                let handle = SkillsHandle {
                    registry: registry.clone(),
                    deps: deps.clone(),
                    dir,
                };
                let (dispatcher_handle, _task) =
                    SkillDispatcher::spawn(registry, event_bus.sender(), shutdown.clone());
                (rules, Some(dispatcher_handle), Some(handle))
            }
            None => ( /* as before, plus None for the handle */ ),
```

(Restructure the `match` to yield a 3-tuple `(rules, dispatcher, skills_handle)`; store `skills: skills_handle` in the returned `Runtime`.) Add `skills: None`/field to the struct and constructor accordingly. Update `crates/athena-voice-cli/src/serve.rs` construction of `SkillsInit` with `disabled: vec![]` for now (Task 11 fills it from the store).

- [ ] **Step 2: Verify it compiles and existing tests pass**

Run: `cargo build --workspace && cargo test -p athena-voice-runtime && cargo clippy --workspace --all-targets`
Expected: green, no warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-runtime crates/athena-voice-cli
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Runtime: expose SkillsHandle and unload disabled skills at startup"
```

---

### Task 7: Admin crate — scaffold, token, auth middleware, /api/status

**Files:**
- Create: `crates/athena-voice-admin/Cargo.toml`
- Create: `crates/athena-voice-admin/src/lib.rs`
- Create: `crates/athena-voice-admin/src/auth.rs`
- Create: `crates/athena-voice-admin/tests/api.rs`
- Modify: `Cargo.toml` (workspace members + deps)

**Interfaces:**
- Consumes: `Store` (Task 3), `SkillsHandle` (Task 6), `SkillConfig`.
- Produces:
  - `athena_voice_admin::AdminDeps { store: Arc<dyn Store>, skills: Option<SkillsHandle>, base_per_skill: HashMap<String, SkillConfig>, token_hash: String, bundled_dir: Option<PathBuf> }`
  - `athena_voice_admin::router(deps: AdminDeps) -> axum::Router`
  - `athena_voice_admin::serve(addr: SocketAddr, deps: AdminDeps) -> anyhow::Result<()>` (binds and runs forever)
  - `athena_voice_admin::auth::ensure_token(store: &Arc<dyn Store>) -> anyhow::Result<Option<String>>` — generates+stores hash on first run, returns the plaintext exactly once
  - `athena_voice_admin::auth::verify(hash: &str, token: &str) -> bool`

- [ ] **Step 1: Workspace wiring**

Root `Cargo.toml`: add `"crates/athena-voice-admin"` to `members`. Add to `[workspace.dependencies]`:

```toml
# admin web UI
axum = { version = "0.8", features = ["multipart"] }
argon2 = { version = "0.5", features = ["std"] }
include_dir = "0.7"
athena-voice-skill-sdk = { path = "crates/athena-voice-skill-sdk" }
```

Create `crates/athena-voice-admin/Cargo.toml`:

```toml
[package]
name = "athena-voice-admin"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[dependencies]
anyhow = { workspace = true }
argon2 = { workspace = true }
axum = { workspace = true }
include_dir = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["net"] }
tracing = { workspace = true }
uuid = { workspace = true }
athena-voice-runtime = { workspace = true }
athena-voice-skill-sdk = { workspace = true }
athena-voice-storage = { workspace = true }

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }

[lints]
workspace = true
```

(`tower` is dev-only, for `Router::oneshot` in tests.)

- [ ] **Step 2: Write the failing tests**

Create `crates/athena-voice-admin/tests/api.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use athena_voice_admin::{AdminDeps, auth, router};
use athena_voice_storage::{SqliteStore, Store};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt; // oneshot

async fn test_deps() -> (AdminDeps, String) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let token = auth::ensure_token(&store).await.unwrap().expect("first run yields a token");
    let hash = store.admin_token_hash().await.unwrap().unwrap();
    (
        AdminDeps {
            store,
            skills: None,
            base_per_skill: HashMap::new(),
            token_hash: hash,
            bundled_dir: None,
        },
        token,
    )
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn status_requires_token() {
    let (deps, token) = test_deps().await;
    let app = router(deps);
    let unauth = app.clone().oneshot(get("/api/status", None)).await.unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    let bad = app.clone().oneshot(get("/api/status", Some("wrong"))).await.unwrap();
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
    let ok = app.oneshot(get("/api/status", Some(&token))).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn ensure_token_is_first_run_only() {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let first = auth::ensure_token(&store).await.unwrap();
    assert!(first.is_some());
    let second = auth::ensure_token(&store).await.unwrap();
    assert!(second.is_none(), "token must not regenerate once stored");
}

#[tokio::test]
async fn index_is_served_without_token() {
    // The static UI itself is public; every /api/* call it makes needs the token.
    let (deps, _) = test_deps().await;
    let app = router(deps);
    let res = app.oneshot(get("/", None)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin`
Expected: compile FAIL (crate skeleton missing).

- [ ] **Step 4: Implement**

`crates/athena-voice-admin/src/auth.rs`:

```rust
//! First-run admin token + Bearer verification.
//!
//! The token is generated once, printed to the terminal by the caller, and
//! only its argon2 hash is stored. There is deliberately no recovery: to
//! reset, delete the `admin_auth` row and restart.

use std::sync::Arc;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use athena_voice_storage::Store;
use uuid::Uuid;

/// On first run: generate a token, store its hash, and return the plaintext
/// (show it to the user immediately — it is never recoverable later).
/// Subsequent runs return `None`.
pub async fn ensure_token(store: &Arc<dyn Store>) -> anyhow::Result<Option<String>> {
    if store.admin_token_hash().await?.is_some() {
        return Ok(None);
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash admin token: {e}"))?
        .to_string();
    store.admin_token_hash_set(&hash).await?;
    Ok(Some(token))
}

pub fn verify(hash: &str, token: &str) -> bool {
    PasswordHash::new(hash)
        .is_ok_and(|h| Argon2::default().verify_password(token.as_bytes(), &h).is_ok())
}
```

`crates/athena-voice-admin/src/lib.rs`:

```rust
#![deny(warnings)]
//! Admin web interface: token-protected JSON API + embedded static UI.

pub mod auth;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::SkillConfig;
use athena_voice_storage::Store;

pub struct AdminDeps {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    /// TOML-derived per-skill config — the merge base; DB rows override it.
    pub base_per_skill: HashMap<String, SkillConfig>,
    pub token_hash: String,
    pub bundled_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    pub base_per_skill: Arc<HashMap<String, SkillConfig>>,
    pub token_hash: Arc<String>,
    pub bundled_dir: Option<PathBuf>,
}

pub fn router(deps: AdminDeps) -> Router {
    let state = AppState {
        store: deps.store,
        skills: deps.skills,
        base_per_skill: Arc::new(deps.base_per_skill),
        token_hash: Arc::new(deps.token_hash),
        bundled_dir: deps.bundled_dir,
    };
    // The api sub-router is fully stated (Router<()>) before nesting; the
    // outer router stays stateless — the asset handler needs no state.
    let api = Router::new()
        .route("/status", get(status))
        // Task 8+ add: skills list/config/enable/upload/bundled routes here.
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state);
    Router::new().nest("/api", api).fallback(get(static_asset))
}

/// Bind and serve forever (spawned as a background task by `serve`).
pub async fn serve(addr: SocketAddr, deps: AdminDeps) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "admin UI listening");
    axum::serve(listener, router(deps)).await?;
    Ok(())
}

async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| auth::verify(&state.token_hash, t));
    if ok {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn status(State(state): State<AppState>) -> Response {
    let loaded = state
        .skills
        .as_ref()
        .map_or(0, |h| h.registry.skill_names().len());
    axum::Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "skills_loaded": loaded,
    }))
    .into_response()
}

/// Placeholder until Task 12 embeds the real UI: serve a stub index so the
/// root URL is 200 from day one.
async fn static_asset() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><title>Athena-Voice</title><p>Admin UI comes in a later task.</p>",
    )
        .into_response()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin crate: axum scaffold with first-run token auth and /api/status"
```

---

### Task 8: Admin API — list skills with masked config (GET /api/skills)

**Files:**
- Create: `crates/athena-voice-admin/src/api.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (mount route, `pub(crate) mod api;`)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: `Store::{skill_settings_for, skills_disabled}`, `SkillsHandle`, `apply_settings`, `ConfigSchema`.
- Produces JSON per skill:

```json
{ "name": "jeedom", "loaded": true, "enabled": true,
  "schema": { "fields": [ … ] } | null,
  "config": { "base_url": {"kind":"plain","value":"http://x"},
              "api_key": {"kind":"secret","set":true} },
  "http_allowlist": ["192.168.1.91"] }
```

The skill list is the union of `*.wasm` stems in the skills dir, registry names, and skills having DB settings/state — so disabled (unloaded) skills still appear.

- [ ] **Step 1: Write the failing test**

Append to `tests/api.rs`:

```rust
#[tokio::test]
async fn skills_list_masks_secrets_and_shows_disabled() {
    let (mut deps, token) = test_deps().await;
    // Base TOML config for a skill that is not loaded (skills: None).
    let mut base = HashMap::new();
    base.insert(
        "jeedom".to_string(),
        athena_voice_runtime::wasm::registry::SkillConfig {
            http_allowlist: vec!["192.168.1.91".into()],
            config: HashMap::from([("base_url".into(), "http://toml".into())]),
            ..Default::default()
        },
    );
    deps.base_per_skill = base;
    deps.store.skill_setting_set("jeedom", "api_key", "s3cret", true).await.unwrap();
    deps.store.skill_setting_set("jeedom", "base_url", "http://db", false).await.unwrap();
    deps.store.skill_enabled_set("jeedom", false).await.unwrap();

    let app = router(deps);
    let res = app.oneshot(get("/api/skills", Some(&token))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!body.to_string().contains("s3cret"), "secret value must never be echoed");

    let jeedom = body.as_array().unwrap().iter()
        .find(|s| s["name"] == "jeedom").expect("jeedom listed even though unloaded");
    assert_eq!(jeedom["enabled"], false);
    assert_eq!(jeedom["loaded"], false);
    assert_eq!(jeedom["config"]["api_key"]["kind"], "secret");
    assert_eq!(jeedom["config"]["api_key"]["set"], true);
    assert_eq!(jeedom["config"]["base_url"]["value"], "http://db"); // DB wins
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p athena-voice-admin skills_list`
Expected: FAIL (404 — route missing).

- [ ] **Step 3: Implement**

Create `crates/athena-voice-admin/src/api.rs`:

```rust
//! JSON handlers for the admin API.

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use athena_voice_runtime::wasm::settings::{HTTP_ALLOWLIST_KEY, apply_settings};
use athena_voice_skill_sdk::ConfigSchema;

use crate::AppState;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum ConfigValue {
    Plain { value: String },
    Secret { set: bool },
}

#[derive(Serialize)]
pub(crate) struct SkillInfo {
    name: String,
    loaded: bool,
    enabled: bool,
    schema: Option<ConfigSchema>,
    config: BTreeMap<String, ConfigValue>,
    http_allowlist: Vec<String>,
}

pub(crate) fn internal_error(e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, "admin api error");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()})))
        .into_response()
}

/// Union of: wasm files on disk, loaded registry names, names in the DB.
async fn known_skill_names(state: &AppState) -> anyhow::Result<BTreeSet<String>> {
    let mut names: BTreeSet<String> = state.base_per_skill.keys().cloned().collect();
    if let Some(handle) = &state.skills {
        names.extend(handle.registry.skill_names());
        if let Ok(entries) = std::fs::read_dir(&handle.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    names.insert(stem.to_string());
                }
            }
        }
    }
    for row in state.store.skill_settings_all().await? {
        names.insert(row.skill);
    }
    for name in state.store.skills_disabled().await? {
        names.insert(name);
    }
    Ok(names)
}

pub(crate) async fn skill_info(state: &AppState, name: &str) -> anyhow::Result<SkillInfo> {
    let rows = state.store.skill_settings_for(name).await?;
    let secret_keys: BTreeSet<String> = rows
        .iter()
        .filter(|r| r.is_secret)
        .map(|r| r.key.clone())
        .collect();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let base = state.base_per_skill.get(name).cloned().unwrap_or_default();
    let merged = apply_settings(&base, &pairs);

    let (loaded, schema) = match &state.skills {
        Some(h) => (
            h.registry.skill_names().contains(&name.to_string()),
            h.registry.config_schema(name),
        ),
        None => (false, None),
    };
    // Schema marks secrets too, even when the value still comes from TOML.
    let schema_secret_keys: BTreeSet<String> = schema
        .as_ref()
        .map(|s| s.fields.iter().filter(|f| f.is_secret()).map(|f| f.key.clone()).collect())
        .unwrap_or_default();

    let disabled = state.store.skills_disabled().await?;
    let config = merged
        .config
        .iter()
        .filter(|(k, _)| k.as_str() != HTTP_ALLOWLIST_KEY)
        .map(|(k, v)| {
            let value = if secret_keys.contains(k) || schema_secret_keys.contains(k) {
                ConfigValue::Secret { set: !v.is_empty() }
            } else {
                ConfigValue::Plain { value: v.clone() }
            };
            (k.clone(), value)
        })
        .collect();

    Ok(SkillInfo {
        name: name.to_string(),
        loaded,
        enabled: !disabled.contains(&name.to_string()),
        schema,
        config,
        http_allowlist: merged.http_allowlist,
    })
}

pub(crate) async fn list_skills(State(state): State<AppState>) -> Response {
    let names = match known_skill_names(&state).await {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        match skill_info(&state, &name).await {
            Ok(info) => out.push(info),
            Err(e) => return internal_error(e),
        }
    }
    Json(out).into_response()
}
```

In `lib.rs`: add `pub(crate) mod api;` and mount `.route("/skills", get(api::list_skills))` in the `/api` router.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin API: list skills with schemas and secret-masked config"
```

---

### Task 9: Admin API — validated config writes with live reload (PUT /api/skills/{name}/config)

**Files:**
- Create: `crates/athena-voice-admin/src/validate.rs`
- Modify: `crates/athena-voice-admin/src/api.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (route)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Request body: `{"values": {"<key>": "<string value>", …}}` — list fields arrive as JSON-encoded strings (exactly what `host_config_get` serves the guest). Keys absent from `values` are left unchanged (that's how the UI avoids re-sending untouched secrets).
- Response 200: `{"ok": true, "reload_error": null | "<message>"}` (config persists even if hot reload fails); 400: `{"error": "<message>"}` on validation failure.
- Produces: `validate::validate(schema: Option<&ConfigSchema>, values: &HashMap<String, String>) -> Result<(), String>` and `validate::derived_allowlist(schema: &ConfigSchema, merged_values: &HashMap<String, String>) -> Vec<String>`.

- [ ] **Step 1: Write the failing validator tests**

Create `crates/athena-voice-admin/src/validate.rs` starting with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use athena_voice_skill_sdk::{ConfigField, ConfigSchema, FieldKind, ItemField};
    use std::collections::HashMap;

    fn field(key: &str, kind: FieldKind, required: bool) -> ConfigField {
        ConfigField {
            key: key.into(),
            label: key.into(),
            kind,
            required,
            help: String::new(),
            default: String::new(),
            item_fields: vec![],
        }
    }

    fn jeedom_schema() -> ConfigSchema {
        let mut sensors = field("sensors", FieldKind::List, false);
        sensors.item_fields = vec![
            ItemField { key: "name".into(), kind: FieldKind::String },
            ItemField { key: "id".into(), kind: FieldKind::Number },
            ItemField { key: "unit".into(), kind: FieldKind::String },
        ];
        ConfigSchema {
            fields: vec![
                field("base_url", FieldKind::Url, true),
                field("api_key", FieldKind::Secret, true),
                sensors,
            ],
        }
    }

    #[test]
    fn accepts_valid_values_and_derives_allowlist() {
        let values = HashMap::from([
            ("base_url".to_string(), "http://192.168.1.91".to_string()),
            ("api_key".to_string(), "k".to_string()),
            ("sensors".to_string(), r#"[{"name":"salon","id":142,"unit":"degrés"}]"#.to_string()),
        ]);
        assert_eq!(validate(Some(&jeedom_schema()), &values), Ok(()));
        assert_eq!(derived_allowlist(&jeedom_schema(), &values), vec!["192.168.1.91"]);
    }

    #[test]
    fn rejects_bad_inputs() {
        let s = jeedom_schema();
        let bad_url = HashMap::from([("base_url".to_string(), "192.168.1.91".to_string())]);
        assert!(validate(Some(&s), &bad_url).is_err(), "url must carry a scheme");

        let bad_list = HashMap::from([("sensors".to_string(), r#"[{"name":"x","id":"142"}]"#.to_string())]);
        assert!(validate(Some(&s), &bad_list).is_err(), "id must be a JSON number");

        let missing_required = HashMap::from([("base_url".to_string(), "  ".to_string())]);
        assert!(validate(Some(&s), &missing_required).is_err(), "required present-but-blank rejected");

        let host_schema = ConfigSchema { fields: vec![field("host", FieldKind::Host, true)] };
        let bad_host = HashMap::from([("host".to_string(), "http://x/y".to_string())]);
        assert!(validate(Some(&host_schema), &bad_host).is_err(), "host must be bare");
    }

    #[test]
    fn no_schema_means_no_validation() {
        let values = HashMap::from([("anything".to_string(), "goes".to_string())]);
        assert_eq!(validate(None, &values), Ok(()));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin validate`
Expected: compile FAIL.

- [ ] **Step 3: Implement the validator**

Prepend to `validate.rs`:

```rust
//! Schema-driven validation for config writes; keys absent from `values`
//! are not validated (absent means "unchanged").

use std::collections::HashMap;

use athena_voice_skill_sdk::{ConfigSchema, FieldKind, ItemField};

pub(crate) fn validate(
    schema: Option<&ConfigSchema>,
    values: &HashMap<String, String>,
) -> Result<(), String> {
    let Some(schema) = schema else { return Ok(()) };
    for f in &schema.fields {
        let Some(raw) = values.get(&f.key) else { continue };
        if raw.trim().is_empty() {
            if f.required {
                return Err(format!("`{}` is required", f.key));
            }
            continue;
        }
        check(f.kind, raw, &f.key, &f.item_fields)?;
    }
    Ok(())
}

fn check(kind: FieldKind, raw: &str, key: &str, items: &[ItemField]) -> Result<(), String> {
    match kind {
        FieldKind::String | FieldKind::Secret => Ok(()),
        FieldKind::Number => raw
            .trim()
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("`{key}` must be a number")),
        FieldKind::Url => {
            if raw.starts_with("http://") || raw.starts_with("https://") {
                Ok(())
            } else {
                Err(format!("`{key}` must start with http:// or https://"))
            }
        }
        FieldKind::Host => {
            if raw.contains("://") || raw.contains('/') || raw.contains(' ') {
                Err(format!("`{key}` must be a bare host name (no scheme or path)"))
            } else {
                Ok(())
            }
        }
        FieldKind::List => {
            let parsed: Vec<serde_json::Map<String, serde_json::Value>> =
                serde_json::from_str(raw)
                    .map_err(|e| format!("`{key}` is not a JSON array of objects: {e}"))?;
            for (i, item) in parsed.iter().enumerate() {
                for f in items {
                    let v = item
                        .get(&f.key)
                        .ok_or_else(|| format!("`{key}[{i}]` is missing `{}`", f.key))?;
                    let ok = match f.kind {
                        FieldKind::Number => v.is_number(),
                        _ => v.is_string(),
                    };
                    if !ok {
                        let want = if matches!(f.kind, FieldKind::Number) { "number" } else { "string" };
                        return Err(format!("`{key}[{i}].{}` must be a {want}", f.key));
                    }
                }
            }
            Ok(())
        }
    }
}

/// Hosts implied by the schema's `host` fields plus the host part of `url`
/// fields — becomes the skill's HTTP allowlist so users never edit it by hand.
pub(crate) fn derived_allowlist(
    schema: &ConfigSchema,
    merged_values: &HashMap<String, String>,
) -> Vec<String> {
    let mut hosts = Vec::new();
    for f in &schema.fields {
        let Some(raw) = merged_values.get(&f.key) else { continue };
        if raw.trim().is_empty() {
            continue;
        }
        match f.kind {
            FieldKind::Host => hosts.push(raw.trim().to_string()),
            FieldKind::Url => {
                let no_scheme = raw.split("://").nth(1).unwrap_or(raw);
                let host = no_scheme.split('/').next().unwrap_or("");
                let host = host.split(':').next().unwrap_or("");
                if !host.is_empty() {
                    hosts.push(host.to_string());
                }
            }
            _ => {}
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}
```

- [ ] **Step 4: Write the failing endpoint test**

Append to `tests/api.rs`:

```rust
#[tokio::test]
async fn put_config_persists_and_rejects_invalid() {
    let (deps, token) = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);

    // No schema (skill not loaded) → free-form values accepted.
    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"values":{"base_url":"http://192.168.1.91"}}"#))
        .unwrap();
    let res = app.clone().oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let rows = store.skill_settings_for("jeedom").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "http://192.168.1.91");

    // Malformed body → 400.
    let bad = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"nope": 1}"#))
        .unwrap();
    let res = app.oneshot(bad).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY); // axum Json rejection
}
```

Run: `cargo test -p athena-voice-admin put_config` — expected FAIL (404).

- [ ] **Step 5: Implement the endpoint**

Append to `api.rs`:

```rust
use std::collections::HashMap;

use axum::extract::Path;
use serde::Deserialize;

use crate::validate::{derived_allowlist, validate};

#[derive(Deserialize)]
pub(crate) struct ConfigWrite {
    values: HashMap<String, String>,
}

pub(crate) async fn put_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<ConfigWrite>,
) -> Response {
    let schema = state
        .skills
        .as_ref()
        .and_then(|h| h.registry.config_schema(&name));
    if let Err(msg) = validate(schema.as_ref(), &body.values) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": msg}))).into_response();
    }
    let secret_keys: std::collections::BTreeSet<&str> = schema
        .as_ref()
        .map(|s| s.fields.iter().filter(|f| f.is_secret()).map(|f| f.key.as_str()).collect())
        .unwrap_or_default();

    for (key, value) in &body.values {
        if key.starts_with('$') {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "keys starting with $ are reserved"})),
            )
                .into_response();
        }
        if let Err(e) = state
            .store
            .skill_setting_set(&name, key, value, secret_keys.contains(key.as_str()))
            .await
        {
            return internal_error(e);
        }
    }

    // Recompute the allowlist from the FULL merged value set (old + new),
    // so saving one field doesn't drop hosts implied by others.
    if let Some(schema) = &schema {
        let rows = match state.store.skill_settings_for(&name).await {
            Ok(r) => r,
            Err(e) => return internal_error(e),
        };
        let base = state.base_per_skill.get(&name).cloned().unwrap_or_default();
        let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
        let merged = apply_settings(&base, &pairs);
        let hosts = derived_allowlist(schema, &merged.config);
        if !hosts.is_empty() {
            let json = serde_json::to_string(&hosts).expect("Vec<String> serializes");
            if let Err(e) = state
                .store
                .skill_setting_set(&name, HTTP_ALLOWLIST_KEY, &json, false)
                .await
            {
                return internal_error(e);
            }
        }
    }

    let reload_error = reload_skill(&state, &name).await.err();
    Json(serde_json::json!({"ok": true, "reload_error": reload_error})).into_response()
}

/// Rebuild the skill's merged config and reload its plugin in place.
/// `Ok(())` when the admin runs without a skill runtime (config still saved).
pub(crate) async fn reload_skill(state: &AppState, name: &str) -> Result<(), String> {
    let Some(handle) = &state.skills else { return Ok(()) };
    let wasm = handle.dir.join(format!("{name}.wasm"));
    if !wasm.is_file() {
        return Ok(()); // not installed yet; config waits for the upload
    }
    let rows = state
        .store
        .skill_settings_for(name)
        .await
        .map_err(|e| e.to_string())?;
    let base = state.base_per_skill.get(name).cloned().unwrap_or_default();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let merged = apply_settings(&base, &pairs);

    let mut deps = handle.deps.clone();
    deps.per_skill.insert(name.to_string(), merged);
    // reload_path is synchronous plugin construction — run it off the
    // async worker thread.
    let registry = handle.registry.clone();
    let res = tokio::task::spawn_blocking(move || registry.reload_path(&wasm, &deps))
        .await
        .map_err(|e| e.to_string())?;
    res.map(|_| ()).map_err(|e| e.to_string())
}
```

In `lib.rs`, mount: `.route("/skills/{name}/config", axum::routing::put(api::put_config))` and add `pub(crate) mod validate;`.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin API: validated config writes with live skill reload"
```

---

### Task 10: Admin API — enable/disable and upload/install

**Files:**
- Modify: `crates/athena-voice-admin/src/api.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (routes)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- `POST /api/skills/{name}/enable` → persist flag, reload the skill; `POST /api/skills/{name}/disable` → persist flag, `registry.remove(name)`. Both return `{"ok": true, "reload_error": …}`.
- `POST /api/skills/upload` — multipart field `file` with filename `<name>.wasm`; writes `dir/<name>.wasm`, reloads. 32 MiB body cap. Rejects names that aren't `[a-z0-9_-]+`.
- `GET /api/bundled` → `[{"name": "weather"}, …]` from `bundled_dir` (empty when unset); `POST /api/bundled/{name}/install` copies `bundled_dir/<name>.wasm` into the skills dir and reloads.

- [ ] **Step 1: Write the failing tests**

Append to `tests/api.rs`:

```rust
fn post(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn enable_disable_toggles_state() {
    let (deps, token) = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);
    let res = app.clone().oneshot(post("/api/skills/timer/disable", &token)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(store.skills_disabled().await.unwrap(), vec!["timer".to_string()]);
    let res = app.oneshot(post("/api/skills/timer/enable", &token)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(store.skills_disabled().await.unwrap().is_empty());
}

#[tokio::test]
async fn upload_rejects_bad_names() {
    let (deps, token) = test_deps().await;
    let app = router(deps);
    let body = concat!(
        "--BOUND\r\n",
        "Content-Disposition: form-data; name=\"file\"; filename=\"../evil.wasm\"\r\n",
        "Content-Type: application/wasm\r\n\r\n",
        "AGFzbQ\r\n",
        "--BOUND--\r\n",
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/upload")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
```

Run: `cargo test -p athena-voice-admin enable_disable upload_rejects` — expected FAIL (404).

- [ ] **Step 2: Implement**

Append to `api.rs`:

```rust
use axum::extract::Multipart;

pub(crate) async fn enable_skill(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if let Err(e) = state.store.skill_enabled_set(&name, true).await {
        return internal_error(e);
    }
    let reload_error = reload_skill(&state, &name).await.err();
    Json(serde_json::json!({"ok": true, "reload_error": reload_error})).into_response()
}

pub(crate) async fn disable_skill(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if let Err(e) = state.store.skill_enabled_set(&name, false).await {
        return internal_error(e);
    }
    if let Some(handle) = &state.skills {
        handle.registry.remove(&name);
    }
    Json(serde_json::json!({"ok": true, "reload_error": null})).into_response()
}

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub(crate) async fn upload_skill(State(state): State<AppState>, mut parts: Multipart) -> Response {
    let Some(handle) = state.skills.clone() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "no skills directory configured"})),
        )
            .into_response();
    };
    while let Ok(Some(field)) = parts.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let file_name = field.file_name().unwrap_or_default().to_string();
        let Some(name) = file_name.strip_suffix(".wasm") else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "file name must end in .wasm"})),
            )
                .into_response();
        };
        if !valid_skill_name(name) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "skill name must match [a-z0-9_-]+"})),
            )
                .into_response();
        }
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return internal_error(e),
        };
        let dest = handle.dir.join(format!("{name}.wasm"));
        if let Err(e) = tokio::fs::write(&dest, &bytes).await {
            return internal_error(e);
        }
        // Mark enabled (an upload is an explicit install) and load it.
        if let Err(e) = state.store.skill_enabled_set(name, true).await {
            return internal_error(e);
        }
        let reload_error = reload_skill(&state, name).await.err();
        return Json(serde_json::json!({"ok": true, "name": name, "reload_error": reload_error}))
            .into_response();
    }
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "missing multipart field `file`"})),
    )
        .into_response()
}

pub(crate) async fn list_bundled(State(state): State<AppState>) -> Response {
    let mut out = Vec::new();
    if let Some(dir) = &state.bundled_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                out.push(serde_json::json!({"name": stem}));
            }
        }
    }
    Json(out).into_response()
}

pub(crate) async fn install_bundled(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if !valid_skill_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "skill name must match [a-z0-9_-]+"})),
        )
            .into_response();
    }
    let (Some(bundled), Some(handle)) = (state.bundled_dir.clone(), state.skills.clone()) else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "bundled skills not configured"})),
        )
            .into_response();
    };
    let src = bundled.join(format!("{name}.wasm"));
    if !src.is_file() {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "unknown bundled skill"})))
            .into_response();
    }
    if let Err(e) = tokio::fs::copy(&src, handle.dir.join(format!("{name}.wasm"))).await {
        return internal_error(e);
    }
    if let Err(e) = state.store.skill_enabled_set(&name, true).await {
        return internal_error(e);
    }
    let reload_error = reload_skill(&state, &name).await.err();
    Json(serde_json::json!({"ok": true, "reload_error": reload_error})).into_response()
}
```

In `lib.rs` mount inside the `/api` router (and add the body-size cap on the whole router):

```rust
        .route("/skills/{name}/enable", axum::routing::post(api::enable_skill))
        .route("/skills/{name}/disable", axum::routing::post(api::disable_skill))
        .route("/skills/upload", axum::routing::post(api::upload_skill))
        .route("/bundled", get(api::list_bundled))
        .route("/bundled/{name}/install", axum::routing::post(api::install_bundled))
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024))
```

Route-order note: axum 0.8 matches the literal `/skills/upload` over the `/skills/{name}/…` capture regardless of registration order, so both can coexist.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets`
Expected: PASS, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin API: enable/disable, wasm upload, bundled skill install"
```

---

### Task 11: Serve wiring — startup merge, token print, admin server spawn

**Files:**
- Modify: `crates/athena-voice-cli/src/serve.rs`
- Modify: `crates/athena-voice-cli/src/config.rs` (optional `[skills] bundled_dir`)
- Modify: `crates/athena-voice-cli/Cargo.toml` (dep on `athena-voice-admin`)
- Modify: `athena.example.toml`, `athena.voice.toml` (document `bundled_dir`, env overrides)

**Interfaces:**
- Consumes: everything above.
- Produces: `serve` prints the first-run token, applies DB settings + disabled list before `Runtime::spawn`, and spawns `athena_voice_admin::serve` on `cfg.server.{host,port}`.

- [ ] **Step 1: Config knob**

`config.rs` — add to `SkillsConfig`:

```rust
    /// Directory of prebuilt `.wasm` artifacts offered by the web UI's
    /// "install bundled skill" picker. Unset hides the picker.
    #[serde(default)]
    pub bundled_dir: Option<PathBuf>,
```

`athena.example.toml` — under `[skills]`, add:

```toml
# Prebuilt skills offered by the web UI's install picker (optional).
# bundled_dir = "/usr/share/athena-voice/bundled-skills"
```

Also add (near the top of `athena.example.toml`, fulfilling the spec's env-override documentation):

```toml
# Any value in this file can be overridden with environment variables:
# ATHENA__SERVER__PORT=9090, ATHENA__MQTT__HOST=broker.local, etc.
# (double underscore separates nesting levels).
```

- [ ] **Step 2: Wire serve.rs**

In `crates/athena-voice-cli/Cargo.toml` add `athena-voice-admin = { path = "../athena-voice-admin" }` (also add it to `[workspace.dependencies]` as a path dep and use `workspace = true`, matching the other internal crates).

In `serve.rs`, after the store is opened and before `Runtime::spawn`:

```rust
    // Web-edited settings override TOML key-by-key; disabled skills are
    // unloaded right after the directory scan.
    let db_settings = store.skill_settings_all().await?;
    let disabled = store.skills_disabled().await?;

    let base_per_skill: std::collections::HashMap<String, RuntimeSkillConfig> = cfg
        .skills
        .per_skill
        .iter()
        .map(|(name, c)| {
            (
                name.clone(),
                RuntimeSkillConfig {
                    http_allowlist: c.http_allowlist.clone(),
                    mqtt_publish_allowlist: c.mqtt_publish_allowlist.clone(),
                    config: c.config.clone(),
                    retention_gc_after_sec: c.retention.gc_after_sec,
                    config_file: c.config_file.clone(),
                },
            )
        })
        .collect();

    let mut merged_per_skill = base_per_skill.clone();
    {
        use athena_voice_runtime::wasm::settings::apply_settings;
        let mut by_skill: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for row in db_settings {
            by_skill.entry(row.skill).or_default().push((row.key, row.value));
        }
        for (skill, rows) in by_skill {
            let base = base_per_skill.get(&skill).cloned().unwrap_or_default();
            merged_per_skill.insert(skill, apply_settings(&base, &rows));
        }
    }
```

Replace the existing `per_skill` mapping inside the `SkillsInit` construction with `per_skill: merged_per_skill` and add `disabled`. Then, after `Runtime::spawn` succeeds:

```rust
    // Admin web UI — first run prints the token exactly once.
    if let Some(token) = athena_voice_admin::auth::ensure_token(&store).await? {
        println!("\n==============================================================");
        println!(" Admin UI token (shown once — save it now):");
        println!("   {token}");
        println!(" Open http://{}:{} and paste it when prompted.", cfg.server.host, cfg.server.port);
        println!("==============================================================\n");
    }
    let token_hash = store
        .admin_token_hash()
        .await?
        .expect("ensure_token guarantees a stored hash");
    let admin_addr: std::net::SocketAddr = format!("{}:{}", cfg.server.host, cfg.server.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid [server] host/port: {e}"))?;
    let admin_deps = athena_voice_admin::AdminDeps {
        store: store.clone(),
        skills: runtime.skills.clone(),
        base_per_skill,
        token_hash,
        bundled_dir: cfg.skills.bundled_dir.clone(),
    };
    drop(tokio::spawn(async move {
        if let Err(e) = athena_voice_admin::serve(admin_addr, admin_deps).await {
            tracing::error!(error = %e, "admin server exited");
        }
    }));
```

Note: `cfg.server.host = "127.0.0.1"` in `athena.voice.toml` already gives the safe default bind; `athena.example.toml` currently says `0.0.0.0` — change the example to `host = "127.0.0.1"` with a comment: `# Set to 0.0.0.0 to reach the web UI from your LAN (token still required).`

`SkillsHandle` must be `Clone` and `Runtime.skills` public (done in Task 6).

- [ ] **Step 3: Verify end-to-end by hand**

Run: `cargo run -p athena-voice-cli -- serve --config athena.voice.toml --dry-run` → exits 0.
Start mosquitto if not running (`brew services start mosquitto`), then:
Run: `cargo run -p athena-voice-cli -- serve --config athena.voice.toml` (background it or use a second terminal)
Expected: startup banner shows the one-time token.
Run: `curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/api/status` → `401`
Run: `curl -s -H "Authorization: Bearer <token>" http://127.0.0.1:8080/api/status` → `{"skills_loaded":…,"version":"0.1.0"}`
Run: `curl -s -H "Authorization: Bearer <token>" http://127.0.0.1:8080/api/skills | head -c 400` → JSON array with the four skills.
Stop the server.

- [ ] **Step 4: Run the full suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: green, no warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/athena-voice-cli athena.example.toml athena.voice.toml
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Serve: spawn admin UI with startup settings merge and one-time token"
```

---

### Task 12: Frontend — embedded three-screen UI

**Files:**
- Create: `crates/athena-voice-admin/static/index.html`
- Create: `crates/athena-voice-admin/static/app.js`
- Create: `crates/athena-voice-admin/static/style.css`
- Modify: `crates/athena-voice-admin/src/lib.rs` (replace the stub `static_asset`)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: every `/api/*` endpoint above; the `SkillInfo` JSON shape from Task 8 verbatim.
- Produces: `GET /` → index.html; `GET /app.js`, `GET /style.css` served from the embedded dir with correct MIME types.

- [ ] **Step 1: Write the failing test**

Append to `tests/api.rs`:

```rust
#[tokio::test]
async fn static_assets_served_with_mime() {
    let (deps, _) = test_deps().await;
    let app = router(deps);
    for (path, mime) in [
        ("/", "text/html; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/style.css", "text/css; charset=utf-8"),
    ] {
        let res = app.clone().oneshot(get(path, None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(),
            mime,
            "{path}"
        );
    }
    let missing = app.oneshot(get("/nope.png", None)).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
```

Run: `cargo test -p athena-voice-admin static_assets` — expected FAIL.

- [ ] **Step 2: Replace the asset handler**

In `lib.rs`:

```rust
use include_dir::{Dir, include_dir};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/static");

async fn static_asset(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = ASSETS.get_file(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
}
```

(keep the `.fallback(get(static_asset))` wiring; delete the stub body).

- [ ] **Step 3: Write the three static files**

`static/index.html`:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Athena-Voice</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <header>
    <h1>Athena-Voice</h1>
    <span id="status"></span>
  </header>
  <main id="app"><!-- rendered by app.js --></main>
  <template id="tpl-token">
    <section class="card">
      <h2 data-i18n="token_title"></h2>
      <p data-i18n="token_help"></p>
      <input id="token-input" type="password" autocomplete="off">
      <button id="token-save" data-i18n="save"></button>
      <p class="error" id="token-error"></p>
    </section>
  </template>
  <script src="/app.js"></script>
</body>
</html>
```

`static/style.css`:

```css
:root {
  --bg: #f6f7f9; --card: #fff; --ink: #1c2733; --muted: #6b7a8c;
  --accent: #2563eb; --danger: #b91c1c; --ok: #15803d; --line: #e3e8ee;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #10161d; --card: #1a222c; --ink: #e8edf2; --muted: #93a3b4;
    --accent: #60a5fa; --danger: #f87171; --ok: #4ade80; --line: #2a3542;
  }
}
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.5 system-ui, sans-serif; background: var(--bg); color: var(--ink); }
header { display: flex; align-items: baseline; gap: 1rem; padding: 1rem 1.5rem; border-bottom: 1px solid var(--line); }
h1 { font-size: 1.1rem; margin: 0; }
#status { color: var(--muted); font-size: .85rem; }
main { max-width: 720px; margin: 1.5rem auto; padding: 0 1rem; display: grid; gap: 1rem; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 1rem 1.25rem; }
.card h2 { margin: 0 0 .5rem; font-size: 1rem; }
.skill-row { display: flex; align-items: center; gap: .75rem; padding: .5rem 0; border-top: 1px solid var(--line); }
.skill-row:first-of-type { border-top: 0; }
.skill-row .name { font-weight: 600; flex: 1; cursor: pointer; }
.badge { font-size: .75rem; padding: .1rem .5rem; border-radius: 999px; border: 1px solid var(--line); color: var(--muted); }
.badge.ok { color: var(--ok); border-color: var(--ok); }
.badge.off { color: var(--danger); border-color: var(--danger); }
label { display: block; margin: .75rem 0 .25rem; font-weight: 600; font-size: .85rem; }
.help { color: var(--muted); font-size: .8rem; margin: .1rem 0 0; }
input[type=text], input[type=password], input[type=number] {
  width: 100%; padding: .45rem .6rem; border: 1px solid var(--line); border-radius: 6px;
  background: var(--bg); color: var(--ink);
}
button { padding: .45rem .9rem; border: 0; border-radius: 6px; background: var(--accent); color: #fff; cursor: pointer; }
button.quiet { background: transparent; color: var(--accent); }
button.danger { background: var(--danger); }
table { width: 100%; border-collapse: collapse; margin-top: .25rem; }
td, th { padding: .25rem .4rem; border: 1px solid var(--line); font-size: .85rem; }
.error { color: var(--danger); min-height: 1.2em; }
.notice { color: var(--ok); }
.secret-set { color: var(--muted); font-size: .8rem; }
```

`static/app.js`:

```javascript
'use strict';

const T = {
  en: {
    token_title: 'Admin token', save: 'Save', skills: 'Skills',
    token_help: 'Paste the token printed in the terminal on first start.',
    bad_token: 'That token was not accepted.',
    enabled: 'enabled', disabled: 'disabled', loaded: 'loaded', not_loaded: 'not loaded',
    enable: 'Enable', disable: 'Disable', back: '← Back', add_row: 'Add row', remove: 'Remove',
    saved: 'Saved.', reload_failed: 'Saved, but reload failed: ',
    secret_set: 'A value is stored. Leave blank to keep it.',
    upload_title: 'Install a skill', upload_help: 'Drop a .wasm file or pick a bundled skill.',
    install: 'Install', no_settings: 'This skill has no settings.',
    needs_config: 'needs config', key: 'Key', value: 'Value',
  },
  fr: {
    token_title: 'Jeton administrateur', save: 'Enregistrer', skills: 'Compétences',
    token_help: 'Collez le jeton affiché dans le terminal au premier démarrage.',
    bad_token: 'Jeton refusé.',
    enabled: 'activée', disabled: 'désactivée', loaded: 'chargée', not_loaded: 'non chargée',
    enable: 'Activer', disable: 'Désactiver', back: '← Retour', add_row: 'Ajouter', remove: 'Retirer',
    saved: 'Enregistré.', reload_failed: 'Enregistré, mais rechargement échoué : ',
    secret_set: 'Une valeur est enregistrée. Laissez vide pour la conserver.',
    upload_title: 'Installer une compétence', upload_help: 'Déposez un fichier .wasm ou choisissez une compétence fournie.',
    install: 'Installer', no_settings: 'Cette compétence n’a aucun réglage.',
    needs_config: 'à configurer', key: 'Clé', value: 'Valeur',
  },
};
const lang = (navigator.language || 'en').startsWith('fr') ? 'fr' : 'en';
const t = (k) => T[lang][k] || k;

const app = document.getElementById('app');
let token = localStorage.getItem('athena-admin-token') || '';

async function api(path, opts = {}) {
  const res = await fetch(path, {
    ...opts,
    headers: { Authorization: `Bearer ${token}`, ...(opts.headers || {}) },
  });
  if (res.status === 401) { renderTokenPrompt(true); throw new Error('unauthorized'); }
  return res;
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'onclick') node.addEventListener('click', v);
    else if (k === 'text') node.textContent = v;
    else node.setAttribute(k, v);
  }
  node.append(...children);
  return node;
}

function renderTokenPrompt(failed = false) {
  app.replaceChildren(document.getElementById('tpl-token').content.cloneNode(true));
  app.querySelectorAll('[data-i18n]').forEach((n) => (n.textContent = t(n.dataset.i18n)));
  if (failed) document.getElementById('token-error').textContent = t('bad_token');
  document.getElementById('token-save').onclick = async () => {
    token = document.getElementById('token-input').value.trim();
    localStorage.setItem('athena-admin-token', token);
    renderList();
  };
}

async function renderList() {
  let skills;
  try {
    skills = await (await api('/api/skills')).json();
  } catch { return; }
  const status = await (await api('/api/status')).json();
  document.getElementById('status').textContent =
    `v${status.version} — ${status.skills_loaded} ${t('loaded')}`;

  const list = el('section', { class: 'card' }, el('h2', { text: t('skills') }));
  for (const s of skills) {
    const badges = [
      el('span', { class: `badge ${s.enabled ? 'ok' : 'off'}`, text: s.enabled ? t('enabled') : t('disabled') }),
      el('span', { class: `badge ${s.loaded ? 'ok' : ''}`, text: s.loaded ? t('loaded') : t('not_loaded') }),
    ];
    if (s.schema && s.schema.fields.some((f) => f.required) && Object.keys(s.config).length === 0) {
      badges.push(el('span', { class: 'badge off', text: t('needs_config') }));
    }
    list.append(el('div', { class: 'skill-row' },
      el('span', { class: 'name', text: s.name, onclick: () => renderDetail(s) }),
      ...badges,
      el('button', {
        class: s.enabled ? 'danger' : '',
        text: s.enabled ? t('disable') : t('enable'),
        onclick: async () => {
          await api(`/api/skills/${s.name}/${s.enabled ? 'disable' : 'enable'}`, { method: 'POST' });
          renderList();
        },
      }),
    ));
  }
  app.replaceChildren(list, await uploadCard());
}

function fieldInput(f, current) {
  if (f.type === 'list') return listEditor(f, current);
  const type = f.type === 'secret' ? 'password' : f.type === 'number' ? 'number' : 'text';
  const input = el('input', { type, id: `f-${f.key}`, autocomplete: 'off' });
  if (current && current.kind === 'plain') input.value = current.value;
  const wrap = el('div', {}, el('label', { for: `f-${f.key}`, text: f.label || f.key }), input);
  if (f.type === 'secret' && current && current.set) {
    wrap.append(el('p', { class: 'secret-set', text: t('secret_set') }));
  }
  if (f.help) wrap.append(el('p', { class: 'help', text: f.help }));
  return wrap;
}

function listEditor(f, current) {
  let rows = [];
  try { rows = current && current.kind === 'plain' ? JSON.parse(current.value) : []; } catch {}
  const table = el('table', { 'data-list': f.key });
  const render = () => {
    table.replaceChildren(
      el('tr', {}, ...f.item_fields.map((c) => el('th', { text: c.key })), el('th')),
      ...rows.map((row, i) => el('tr', {},
        ...f.item_fields.map((c) => {
          const cell = el('input', { type: c.type === 'number' ? 'number' : 'text' });
          cell.value = row[c.key] ?? '';
          cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; };
          return el('td', {}, cell);
        }),
        el('td', {}, el('button', { class: 'quiet', text: t('remove'), onclick: () => { rows.splice(i, 1); render(); } })),
      )),
    );
  };
  render();
  table.getRows = () => rows;
  return el('div', {},
    el('label', { text: f.label || f.key }), table,
    el('button', { class: 'quiet', text: t('add_row'), onclick: () => { rows.push({}); render(); } }),
    f.help ? el('p', { class: 'help', text: f.help }) : '',
  );
}

async function renderDetail(skill) {
  const card = el('section', { class: 'card' },
    el('button', { class: 'quiet', text: t('back'), onclick: renderList }),
    el('h2', { text: skill.name }),
  );
  const msg = el('p', { class: 'error' });
  const fields = skill.schema ? skill.schema.fields
    : Object.keys(skill.config).map((k) => ({ key: k, label: k, type: 'string', item_fields: [] }));
  if (skill.schema && fields.length === 0) {
    card.append(el('p', { class: 'help', text: t('no_settings') }));
  }
  const widgets = fields.map((f) => { const w = fieldInput(f, skill.config[f.key]); card.append(w); return [f, w]; });
  card.append(el('button', {
    text: t('save'),
    onclick: async () => {
      const values = {};
      for (const [f, w] of widgets) {
        if (f.type === 'list') {
          values[f.key] = JSON.stringify(w.querySelector('table').getRows());
        } else {
          const v = w.querySelector('input').value;
          if (f.type === 'secret' && v === '') continue; // blank secret = unchanged
          values[f.key] = v;
        }
      }
      const res = await api(`/api/skills/${skill.name}/config`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ values }),
      });
      const body = await res.json();
      if (!res.ok) { msg.textContent = body.error; msg.className = 'error'; return; }
      msg.textContent = body.reload_error ? t('reload_failed') + body.reload_error : t('saved');
      msg.className = body.reload_error ? 'error' : 'notice';
    },
  }), msg);
  app.replaceChildren(card);
}

async function uploadCard() {
  const card = el('section', { class: 'card' },
    el('h2', { text: t('upload_title') }),
    el('p', { class: 'help', text: t('upload_help') }),
  );
  const msg = el('p', { class: 'error' });
  const file = el('input', { type: 'file', accept: '.wasm' });
  card.append(file, el('button', {
    text: t('install'),
    onclick: async () => {
      if (!file.files.length) return;
      const form = new FormData();
      form.append('file', file.files[0]);
      const res = await api('/api/skills/upload', { method: 'POST', body: form });
      const body = await res.json();
      msg.textContent = res.ok
        ? (body.reload_error ? t('reload_failed') + body.reload_error : t('saved'))
        : body.error;
      msg.className = res.ok && !body.reload_error ? 'notice' : 'error';
      if (res.ok) renderList();
    },
  }));
  try {
    const bundled = await (await api('/api/bundled')).json();
    for (const b of bundled) {
      card.append(el('div', { class: 'skill-row' },
        el('span', { class: 'name', text: b.name }),
        el('button', {
          text: t('install'),
          onclick: async () => { await api(`/api/bundled/${b.name}/install`, { method: 'POST' }); renderList(); },
        }),
      ));
    }
  } catch {}
  card.append(msg);
  return card;
}

if (token) renderList(); else renderTokenPrompt();
```

- [ ] **Step 4: Run tests and check by hand**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets` → green.
Start `serve` (broker running), open `http://127.0.0.1:8080`, paste the token: the skill list must render, weather/jeedom detail forms must open, and saving a jeedom config with a sensors row must return "Saved." (jeedom schema arrives in Task 13; before that jeedom shows the key/value fallback).

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: embedded three-screen frontend (list, config forms, install)"
```

---

### Task 13: Bundled skill schemas + rebuilt wasm

**Files:**
- Modify: `skills-jeedom/src/lib.rs`, `skills-weather/src/lib.rs`, `skills-timer/src/lib.rs`, `skills-home/src/lib.rs`
- Modify (binary artifacts): `skills/jeedom.wasm`, `skills/weather.wasm`, `skills/timer.wasm` (note: `skills/` has no `home.wasm` today — build it only if `skills-home/build.sh` targets it; otherwise skip home's binary)

**Interfaces:**
- Consumes: `athena_voice_skill_sdk::{ConfigSchema, ConfigField, FieldKind, ItemField}`.
- Produces: each bundled skill exports `config_schema(String) -> String` (JSON of `ConfigSchema`).

- [ ] **Step 1: Add the export to skills-jeedom**

In `skills-jeedom/src/lib.rs` (alongside the existing `plugin_fn`s), add:

```rust
use athena_voice_skill_sdk::{ConfigField, ConfigSchema, FieldKind, ItemField};

#[plugin_fn]
pub fn config_schema(_input: String) -> FnResult<String> {
    let schema = ConfigSchema {
        fields: vec![
            ConfigField {
                key: "base_url".into(),
                label: "Jeedom URL".into(),
                kind: FieldKind::Url,
                required: true,
                help: "e.g. http://192.168.1.91 — the host is allowed for HTTP automatically".into(),
                default: String::new(),
                item_fields: vec![],
            },
            ConfigField {
                key: "api_key".into(),
                label: "API key".into(),
                kind: FieldKind::Secret,
                required: true,
                help: "Jeedom → Settings → System → Configuration → API".into(),
                default: String::new(),
                item_fields: vec![],
            },
            ConfigField {
                key: "sensors".into(),
                label: "Sensors".into(),
                kind: FieldKind::List,
                required: false,
                help: "Spoken name → Jeedom command id".into(),
                default: String::new(),
                item_fields: vec![
                    ItemField { key: "name".into(), kind: FieldKind::String },
                    ItemField { key: "id".into(), kind: FieldKind::Number },
                    ItemField { key: "unit".into(), kind: FieldKind::String },
                ],
            },
        ],
    };
    Ok(serde_json::to_string(&schema)?)
}
```

- [ ] **Step 2: Same pattern for the other three**

- `skills-weather`: one field — `default_city` (`FieldKind::String`, required, help "City used when none is spoken").
- `skills-timer`: `ConfigSchema { fields: vec![] }` (renders as "no settings" — still better than the fallback editor because the UI knows it's intentional).
- `skills-home`: one `List` field `entities`, item fields `name` (string), `room` (string), `kind` (string), `set_topic` (string), `on_payload` (string), `off_payload` (string) — matching the keys its existing config parsing reads (verify against `skills-home/src/lib.rs` before writing; adjust item keys to the real ones).

Each crate already path-depends on the SDK (they use `athena_voice_skill_sdk::…`); if any lacks `serde_json`, add it to that crate's `Cargo.toml`.

- [ ] **Step 3: Rebuild the wasm binaries**

Run (once): `rustup target add wasm32-wasip1`
Run: `./skills-jeedom/build.sh && ./skills-weather/build.sh && ./skills-timer/build.sh`
(and `./skills-home/build.sh` if it exists and `skills/home.wasm` is part of the repo layout).
Expected: each prints "Copied to ../skills/<name>.wasm".

- [ ] **Step 4: Verify the export end-to-end**

Start `serve` (broker running) and run:
`curl -s -H "Authorization: Bearer <token>" http://127.0.0.1:8080/api/skills | python3 -m json.tool | grep -A3 '"schema"'`
Expected: jeedom/weather entries carry non-null `schema`; open the UI and confirm the jeedom form renders URL/password/table widgets.

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom skills-weather skills-timer skills-home skills
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Bundled skills declare config schemas for the admin UI"
```

---

### Task 14: End-to-end upload test + README documentation

**Files:**
- Modify: `crates/athena-voice-admin/tests/api.rs`
- Modify: `README.md`

**Interfaces:**
- Consumes: everything.

- [ ] **Step 1: Write the e2e upload test**

Append to `tests/api.rs`. Constructing a real `SkillsHandle` needs a `SkillDeps`; an MQTT `AsyncClient` can be created without a running broker (nothing connects until its event loop is polled — the registry only publishes when a skill acts):

```rust
use std::path::PathBuf;

use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::{SkillDeps, SkillRegistry};
use tokio::sync::broadcast;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

async fn deps_with_skills_dir(store: Arc<dyn Store>, dir: PathBuf) -> AdminDeps {
    let (mqtt, _event_loop) = rumqttc::AsyncClient::new(
        rumqttc::MqttOptions::new("admin-test", "127.0.0.1", 1883),
        16,
    );
    let (audio_tx, _rx) = broadcast::channel(8);
    let skill_deps = SkillDeps {
        store: store.clone(),
        mqtt,
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        locales: vec!["fr".into(), "en".into()],
        per_skill: HashMap::new(),
        event_tx: None,
        audio_event_tx: audio_tx,
    };
    let registry = Arc::new(SkillRegistry::new());
    let hash = store.admin_token_hash().await.unwrap().unwrap();
    AdminDeps {
        store,
        skills: Some(SkillsHandle { registry, deps: skill_deps, dir }),
        base_per_skill: HashMap::new(),
        token_hash: hash,
        bundled_dir: None,
    }
}

#[tokio::test]
async fn upload_installs_and_loads_a_real_skill() {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let token = auth::ensure_token(&store).await.unwrap().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let deps = deps_with_skills_dir(store, dir.path().to_path_buf()).await;
    let registry = deps.skills.as_ref().unwrap().registry.clone();
    let app = router(deps);

    let wasm = std::fs::read(repo_root().join("skills/smoke-test.wasm"))
        .expect("committed smoke-test.wasm present");
    let mut body = Vec::new();
    body.extend_from_slice(b"--BOUND\r\nContent-Disposition: form-data; name=\"file\"; filename=\"smoke-test.wasm\"\r\nContent-Type: application/wasm\r\n\r\n");
    body.extend_from_slice(&wasm);
    body.extend_from_slice(b"\r\n--BOUND--\r\n");
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/upload")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["ok"], true);
    assert!(out["reload_error"].is_null(), "reload failed: {out}");
    assert!(registry.skill_names().contains(&"smoke-test".to_string()));
    assert!(dir.path().join("smoke-test.wasm").is_file());
}
```

Dev-dependencies to add in the admin crate's `Cargo.toml`: `tempfile = "3"`, `rumqttc = { workspace = true }`, `reqwest = { workspace = true }`, `tokio-stream`? no — just `tokio = { workspace = true, features = ["net", "rt-multi-thread"] }` if not already. Note: `smoke-test` contains a `-`, which `valid_skill_name` allows.

Caveat for the implementer: if `SkillRegistry::new()` is not `pub`, or `SkillDeps` fields have changed, adapt to the real signatures — the assertion set is the contract, not the setup incantation.

- [ ] **Step 2: Run the test**

Run: `cargo test -p athena-voice-admin upload_installs`
Expected: PASS (fix the setup if signatures drifted).

- [ ] **Step 3: Document in README**

Add a `## Web configuration` section to `README.md` after the quickstart material:

```markdown
## Web configuration

`serve` now hosts a small admin UI on `[server] host/port`
(default `http://127.0.0.1:8080`).

- **First start prints a one-time admin token** — save it; only its hash is
  stored. To reset it, delete the `admin_auth` row in the SQLite DB and
  restart.
- Configure skills (including the Jeedom API key and sensor list) in the
  browser: values land in the SQLite database, **never in TOML files**, and
  override `[skills.<name>]` TOML keys one by one.
- Enable/disable skills and upload new `.wasm` skills from the same page;
  changes apply live, no restart.
- To reach the UI from another machine, set `[server] host = "0.0.0.0"` —
  the token is still required.
- Any config value can also be overridden by environment variables:
  `ATHENA__SERVER__PORT=9090` (double underscore = nesting).

Skills can describe their settings by exporting `config_schema` (see
`skills-jeedom/src/lib.rs`); skills without it get a raw key/value editor.
```

Also update the README's Jeedom setup section: replace the "paste the key into `athena.voice.toml`" instructions with "configure via the web UI"; keep the command-id discovery steps.

- [ ] **Step 4: Full verification**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets && cargo build --workspace`
Expected: all green, no warnings.
Run: `git grep "JJ5qGwlquxyayFlfqYc5" || echo CLEAN` → `CLEAN`.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin README.md Cargo.lock
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin: end-to-end upload test and web configuration docs"
```
