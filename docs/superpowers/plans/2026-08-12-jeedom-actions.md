# Jeedom On/Off Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Voice-triggered on/off device control through the Jeedom skill — discovery, per-device confirmation checkbox, and execution over Jeedom's existing HTTP API.

**Architecture:** The skill (`skills-jeedom`) gains an `actions` config list, `jeedom.turn_on.*`/`turn_off.*`/`confirm`/`cancel` intents, and executes via the same `type=cmd&id=` GET used for reads. Confirmation state lives in the skill's tmp KV. The admin crate pairs discovered on/off action commands into devices and the UI adds them like sensors. One host fix: `fetch_json` must tolerate non-JSON bodies (Jeedom answers action executions with plain text). Spec: `docs/superpowers/specs/2026-08-12-jeedom-actions-design.md`.

**Tech Stack:** Rust (wasm skill via extism-pdk + skill SDK, axum admin, runtime host fns), vanilla-JS admin UI.

## Global Constraints

- Rust files are `#![deny(warnings)]` — code must be warning-free; run `cargo fmt` before every commit.
- Git identity: commit with `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- If cargo complains the pinned toolchain is missing, prefix commands with `RUSTUP_TOOLCHAIN=1.95`.
- `skills-jeedom` is a standalone crate (own `[workspace]`): test it with `cargo test --manifest-path skills-jeedom/Cargo.toml`.
- The runtime's build.rs rebuilds `JEEDOM_TEST_WASM` from `skills-jeedom/src` automatically — workspace `cargo test` picks up skill changes without manual wasm builds.
- Spoken strings are exactly the spec's: FR "C'est fait." / "Tu confirmes : {label} ?" / "Rien à confirmer." / "Annulé."; EN "Done." / "Confirm: {label}?" / "Nothing to confirm." / "Cancelled.".

---

### Task 1: `fetch_json` tolerates non-JSON bodies

**Files:**
- Modify: `crates/athena-voice-runtime/src/wasm/host_fns.rs` (fn `fetch_json`, ~line 464)

**Interfaces:**
- Consumes: nothing new.
- Produces: `fetch_json` returns `Ok(serde_json::Value::String(body))` for a 2xx body that isn't valid JSON (lossy UTF-8, trimmed). Transport errors and non-2xx statuses still `Err`. Task 4's `exec_cmd` relies on a plain-text "ok"/empty action reply arriving as `Ok(_)`.

- [ ] **Step 1: Write the failing test**

`host_fns.rs` has no tests module yet — append one at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    async fn serve_body(body: &'static str) -> wiremock::MockServer {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn fetch_json_parses_json_bodies() {
        let server = serve_body(r#"{"value": 21.5}"#).await;
        let url = reqwest::Url::parse(&server.uri()).unwrap();
        let v = fetch_json(&reqwest::Client::new(), url).await.unwrap();
        assert_eq!(v["value"], 21.5);
    }

    #[tokio::test]
    async fn fetch_json_wraps_plain_text_bodies_as_string() {
        // Jeedom answers an action execution with plain text ("ok") or an
        // empty body — that is a SUCCESS, not a decode error.
        let server = serve_body("ok").await;
        let url = reqwest::Url::parse(&server.uri()).unwrap();
        let v = fetch_json(&reqwest::Client::new(), url).await.unwrap();
        assert_eq!(v, serde_json::Value::String("ok".into()));

        let empty = serve_body("").await;
        let url = reqwest::Url::parse(&empty.uri()).unwrap();
        let v = fetch_json(&reqwest::Client::new(), url).await.unwrap();
        assert_eq!(v, serde_json::Value::String(String::new()));
    }

    #[tokio::test]
    async fn fetch_json_still_errors_on_http_failure() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let url = reqwest::Url::parse(&server.uri()).unwrap();
        assert!(
            fetch_json(&reqwest::Client::new(), url).await.is_err(),
            "non-2xx must stay an error"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify the new behavior fails**

Run: `cargo test -p athena-voice-runtime --lib fetch_json`
Expected: `fetch_json_wraps_plain_text_bodies_as_string` FAILS ("error decoding response body"); the other two PASS.

- [ ] **Step 3: Implement the fallback**

Replace the tail of `fetch_json` (the `resp.json::<serde_json::Value>()…` expression) with:

```rust
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| HostFnError::HttpFailed(redact_query_values(&e.to_string(), &url)))?;
    // Jeedom's simple API answers action executions with plain text or an
    // empty body; surface that as a JSON string rather than a decode error.
    Ok(serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        serde_json::Value::String(String::from_utf8_lossy(&bytes).trim().to_string())
    }))
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-runtime --lib fetch_json`
Expected: all three PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add crates/athena-voice-runtime/src/wasm/host_fns.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Host http_get_json: non-JSON 2xx bodies become JSON strings

Jeedom answers action executions with plain text or an empty body;
treating that as a decode error would make every executed action sound
like a failure.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: `FieldKind::Bool` across SDK, validation, and list editor

**Files:**
- Modify: `crates/athena-voice-skill-sdk/src/schema.rs` (enum `FieldKind`, ~line 31)
- Modify: `crates/athena-voice-admin/src/validate.rs` (fn `check`, ~line 32)
- Modify: `crates/athena-voice-admin/static/app.js` (fn `listEditor`, ~line 306)

**Interfaces:**
- Consumes: nothing new.
- Produces: `FieldKind::Bool` (serialized `"bool"`); list-item validation accepts only JSON booleans for bool item fields; the admin list editor renders a checkbox cell for `c.type === 'bool'` writing a real boolean into the row. Task 3's schema and Task 6's UI rely on all three.

- [ ] **Step 1: Write the failing validation test**

Append to the `tests` module in `crates/athena-voice-admin/src/validate.rs` (it already has `field`/`item_field` helpers):

```rust
    #[test]
    fn list_bool_item_accepts_booleans_only() {
        let mut actions = field("actions", FieldKind::List, false);
        actions.item_fields = vec![
            item_field("on_id", FieldKind::Number, true),
            item_field("confirm", FieldKind::Bool, false),
        ];
        let schema = ConfigSchema { fields: vec![actions] };

        let ok = HashMap::from([(
            "actions".to_string(),
            r#"[{"on_id":124,"confirm":true}]"#.to_string(),
        )]);
        assert!(validate(&schema, &ok).is_ok());

        let bad = HashMap::from([(
            "actions".to_string(),
            r#"[{"on_id":124,"confirm":"yes"}]"#.to_string(),
        )]);
        let err = validate(&schema, &bad).unwrap_err();
        assert!(err.contains("confirm"), "got: {err}");
    }
```

(Match the existing tests' way of calling `validate` — same argument shapes as the neighbouring tests in that module.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p athena-voice-admin --lib validate`
Expected: FAIL to compile — `FieldKind` has no `Bool` variant.

- [ ] **Step 3: Implement**

In `crates/athena-voice-skill-sdk/src/schema.rs`, extend the enum:

```rust
pub enum FieldKind {
    String,
    Number,
    Secret,
    Url,
    Host,
    List,
    Bool,
}
```

In `crates/athena-voice-admin/src/validate.rs` `check()`:
- Add a top-level arm next to `FieldKind::String | FieldKind::Secret`:

```rust
        FieldKind::Bool => match raw.trim() {
            "" | "true" | "false" => Ok(()),
            _ => Err(format!("`{key}` must be true or false")),
        },
```

- In the list-item type check, replace the `let ok = match f.kind { … }` block with:

```rust
                    let ok = match f.kind {
                        FieldKind::Number => v.is_number(),
                        FieldKind::Bool => v.is_boolean(),
                        _ => v.is_string(),
                    };
                    if !ok {
                        let want = match f.kind {
                            FieldKind::Number => "number",
                            FieldKind::Bool => "boolean",
                            _ => "string",
                        };
                        return Err(format!("`{key}[{i}].{}` must be a {want}", f.key));
                    }
```

In `crates/athena-voice-admin/static/app.js` `listEditor`, in the cell-building `else` branch (the `cell = el('input', …)` path), handle bool first:

```js
          } else if (c.type === 'bool') {
            cell = el('input', { type: 'checkbox' });
            cell.checked = row[c.key] === true;
            cell.onchange = () => { row[c.key] = cell.checked; edited(); };
          } else {
```

(keep the existing text/number branch as the final `else`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p athena-voice-admin && cargo test -p athena-voice-skill-sdk && node --check crates/athena-voice-admin/static/app.js`
Expected: PASS / PASS / no output.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add crates/athena-voice-skill-sdk/src/schema.rs crates/athena-voice-admin/src/validate.rs crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Schema: FieldKind::Bool with checkbox list cells and validation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Skill — action config, pattern rules, schema field

**Files:**
- Modify: `skills-jeedom/src/lib.rs`

**Interfaces:**
- Consumes: `FieldKind::Bool` from Task 2.
- Produces: `struct ActionDevice { name: String, room: String, prefix: String, on_id: u64, off_id: u64, confirm: bool }` (Deserialize, room/prefix/confirm `#[serde(default)]`); `fn parse_actions(raw: &str) -> Vec<ActionDevice>`; `fn actions(ctx: &HostCtx) -> &'static [ActionDevice]` (config key `"actions"`, `ACTIONS: OnceCell`); `fn action_rules(locale: &str, devices: &[ActionDevice]) -> Vec<PatternRule>` producing `jeedom.turn_on.{on_id}`, `jeedom.turn_off.{on_id}`, and (when any device has `confirm`) `jeedom.confirm` + `jeedom.cancel`. Task 4 handles those intents; Task 7 asserts the phrases through the registry.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `skills-jeedom/src/lib.rs`:

```rust
    fn a(name: &str, on_id: u64, off_id: u64, confirm: bool) -> ActionDevice {
        ActionDevice {
            name: name.into(),
            room: String::new(),
            prefix: String::new(),
            on_id,
            off_id,
            confirm,
        }
    }

    #[test]
    fn actions_parse_cleans_and_defaults() {
        let raw = r#"[{"name":"lumière 💡 du salon","on_id":124,"off_id":125},
                      {"name":"prise garage","room":"garage","on_id":7,"off_id":8,"confirm":true}]"#;
        let v = parse_actions(raw);
        assert_eq!(v[0].name, "lumière du salon", "symbols stripped");
        assert!(!v[0].confirm, "confirm defaults to false");
        assert!(v[1].confirm);
        assert_eq!(parse_actions("not json"), Vec::<ActionDevice>::new().as_slice());
    }

    #[test]
    fn action_rules_generate_on_off_phrases() {
        let list = vec![a("lumière du salon", 124, 125, false)];
        let rules = action_rules("fr", &list);
        let on = rules.iter().find(|r| r.intent == "jeedom.turn_on.124").unwrap();
        assert!(on.phrases.contains(&"allume la lumière du salon".to_string()), "got: {:?}", on.phrases);
        assert!(on.phrases.contains(&"allume lumière du salon".to_string()));
        let off = rules.iter().find(|r| r.intent == "jeedom.turn_off.124").unwrap();
        assert!(off.phrases.contains(&"éteins la lumière du salon".to_string()), "got: {:?}", off.phrases);

        let en = action_rules("en", &list);
        assert!(en.iter().any(|r| r.intent == "jeedom.turn_on.124"
            && r.phrases.contains(&"turn on the lumière du salon".to_string())));
    }

    #[test]
    fn confirm_rules_only_when_a_device_requires_confirmation() {
        let plain = vec![a("lumière du salon", 124, 125, false)];
        assert!(!action_rules("fr", &plain).iter().any(|r| r.intent == "jeedom.confirm"));

        let confirmed = vec![a("portail", 30, 31, true)];
        let rules = action_rules("fr", &confirmed);
        let confirm = rules.iter().find(|r| r.intent == "jeedom.confirm").unwrap();
        assert!(confirm.phrases.contains(&"oui".to_string()));
        assert!(rules.iter().any(|r| r.intent == "jeedom.cancel"
            && r.phrases.contains(&"annule".to_string())));
    }

    #[test]
    fn no_action_rules_without_devices() {
        assert!(action_rules("fr", &[]).is_empty());
    }
```

For `parse_actions("not json")` comparison, derive `PartialEq` on `ActionDevice`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: FAIL to compile (`ActionDevice` etc. undefined).

- [ ] **Step 3: Implement**

Add after the `Sensor`-related items in `skills-jeedom/src/lib.rs`:

```rust
/// One controllable on/off device: two Jeedom action command ids behind a
/// single spoken name. Like sensors, `name` stores the FULL composed
/// spoken form ("lumière du salon"); room/prefix are auxiliary metadata.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ActionDevice {
    name: String,
    #[serde(default)]
    room: String,
    #[serde(default)]
    prefix: String,
    on_id: u64,
    off_id: u64,
    /// True → the assistant asks "Tu confirmes : … ?" and waits for a
    /// spoken oui/confirme before executing.
    #[serde(default)]
    confirm: bool,
}

static ACTIONS: OnceCell<Vec<ActionDevice>> = OnceCell::new();

fn parse_actions(raw: &str) -> Vec<ActionDevice> {
    let Ok(mut v) = serde_json::from_str::<Vec<ActionDevice>>(raw) else {
        return Vec::new();
    };
    for d in &mut v {
        d.name = clean_spoken(&d.name);
        d.room = clean_spoken(&d.room);
        d.prefix = clean_spoken(&d.prefix.replace('’', "'"));
    }
    v
}

fn actions(ctx: &HostCtx) -> &'static [ActionDevice] {
    ACTIONS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("actions").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            parse_actions(&raw)
        })
        .as_slice()
}

/// Match rules for configured on/off devices, plus the global
/// confirm/cancel rules when any device requires confirmation. The device
/// key riding in the intent name is always `on_id`.
fn action_rules(locale: &str, devices: &[ActionDevice]) -> Vec<PatternRule> {
    if devices.is_empty() {
        return Vec::new();
    }
    let mut rules = Vec::new();
    for d in devices {
        let name = &d.name;
        let (on_phrases, off_phrases): (Vec<String>, Vec<String>) = match locale {
            "fr" => (
                vec![
                    format!("allume la {name}"),
                    format!("allume le {name}"),
                    format!("allume {name}"),
                    format!("active {name}"),
                ],
                vec![
                    format!("éteins la {name}"),
                    format!("éteins le {name}"),
                    format!("éteins {name}"),
                    format!("coupe {name}"),
                    format!("désactive {name}"),
                ],
            ),
            "en" => (
                vec![
                    format!("turn on the {name}"),
                    format!("turn on {name}"),
                    format!("switch on the {name}"),
                ],
                vec![
                    format!("turn off the {name}"),
                    format!("turn off {name}"),
                    format!("switch off the {name}"),
                ],
            ),
            _ => return Vec::new(),
        };
        rules.push(PatternRule {
            intent: format!("jeedom.turn_on.{}", d.on_id),
            phrases: on_phrases,
            slots: Vec::new(),
        });
        rules.push(PatternRule {
            intent: format!("jeedom.turn_off.{}", d.on_id),
            phrases: off_phrases,
            slots: Vec::new(),
        });
    }
    if devices.iter().any(|d| d.confirm) {
        let (yes, no): (Vec<String>, Vec<String>) = match locale {
            "fr" => (
                vec!["oui".into(), "confirme".into(), "c'est confirmé".into()],
                vec!["non".into(), "annule".into()],
            ),
            "en" => (
                vec!["yes".into(), "confirm".into()],
                vec!["no".into(), "cancel".into()],
            ),
            _ => (Vec::new(), Vec::new()),
        };
        rules.push(PatternRule {
            intent: "jeedom.confirm".into(),
            phrases: yes,
            slots: Vec::new(),
        });
        rules.push(PatternRule {
            intent: "jeedom.cancel".into(),
            phrases: no,
            slots: Vec::new(),
        });
    }
    rules
}
```

Wire it into the trait impl — replace the `pattern_rules` body:

```rust
    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        let ctx = HostCtx::for_testing();
        let mut rules = rules_for(locale, sensors(&ctx));
        rules.extend(action_rules(locale, actions(&ctx)));
        rules
    }
```

And extend `config_schema()`'s `fields` vec with (after the `sensors` field):

```rust
            ConfigField {
                key: "actions".into(),
                label: "Actions".into(),
                kind: FieldKind::List,
                required: false,
                help: "On/off devices: spoken name → Jeedom on/off action command ids"
                    .into(),
                default: String::new(),
                item_fields: vec![
                    ItemField {
                        key: "name".into(),
                        kind: FieldKind::String,
                        required: true,
                    },
                    ItemField {
                        key: "on_id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "off_id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "room".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "prefix".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "confirm".into(),
                        kind: FieldKind::Bool,
                        required: false,
                    },
                ],
            },
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS (new tests plus the existing sensor suite).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --manifest-path skills-jeedom/Cargo.toml
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Jeedom skill: action devices — config, schema, turn on/off rules

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Skill — execution and confirmation flow in `handle`

**Files:**
- Modify: `skills-jeedom/src/lib.rs`

**Interfaces:**
- Consumes: `ActionDevice`, `actions()`, intents from Task 3; tolerant `http_get_json` from Task 1; `ctx.tmp_set(key, val, expires_sec)` / `ctx.tmp_get(key)` from the SDK.
- Produces: `handle` responses for `jeedom.turn_on.*`, `jeedom.turn_off.*`, `jeedom.confirm`, `jeedom.cancel`; helpers `fn jeedom_url(ctx: &HostCtx, id: u64) -> Result<String, ()>`, `fn exec_cmd(ctx: &HostCtx, id: u64) -> Result<(), ()>`, `struct Pending { cmd_id: u64, label: String }` (Serialize + Deserialize), `fn action_label(d: &ActionDevice, on: bool, en: bool) -> String`.

- [ ] **Step 1: Write the failing tests**

Append to the skill's `tests` module. Note: on the host target `tmp_get` always returns `Ok(None)` and `config_get_toml` returns `None`, so handle-level tests cover exactly the paths that don't need real state — labels, pending serde, nothing-pending, unknown device.

```rust
    #[test]
    fn action_labels_phrase_both_locales() {
        let d = a("lumière du salon", 124, 125, true);
        assert_eq!(action_label(&d, true, false), "allumer lumière du salon");
        assert_eq!(action_label(&d, false, false), "éteindre lumière du salon");
        assert_eq!(action_label(&d, true, true), "turn on lumière du salon");
        assert_eq!(action_label(&d, false, true), "turn off lumière du salon");
    }

    #[test]
    fn pending_roundtrips_through_json() {
        let p = Pending { cmd_id: 124, label: "allumer lumière du salon".into() };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: Pending = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.cmd_id, 124);
        assert_eq!(back.label, p.label);
    }

    fn speak_text(r: SkillResponse) -> String {
        match r {
            SkillResponse::Speak { text } => text,
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn confirm_with_nothing_pending_says_so() {
        // Host-side tmp_get is always None — exactly the nothing-pending path.
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.confirm".into(),
            slots: Default::default(),
            locale: "fr".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "Rien à confirmer.");
    }

    #[test]
    fn cancel_with_nothing_pending_says_so() {
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.cancel".into(),
            slots: Default::default(),
            locale: "en".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "Nothing to confirm.");
    }

    #[test]
    fn turn_intent_for_unknown_device_apologises() {
        // Host-side config is empty, so no device matches key 999.
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.turn_on.999".into(),
            slots: Default::default(),
            locale: "fr".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "désolé, je ne connais pas cet appareil");
    }
```

`JeedomSkill.handle` takes `&mut self` on a unit struct — call it as `JeedomSkill.handle(...)` exactly like the plugin_fn does. Add `use athena_voice_skill_sdk::Skill as _;` inside the tests module if the trait isn't already in scope, and `Debug` is already derived on `SkillResponse`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: FAIL to compile (`action_label`, `Pending` undefined).

- [ ] **Step 3: Implement**

First refactor the URL construction out of `read_value` — replace its `base`/`api_key`/`url` prelude with a call to the new helper, keeping behavior identical:

```rust
/// Builds the authenticated simple-API URL for one command id.
fn jeedom_url(ctx: &HostCtx, id: u64) -> Result<String, ()> {
    let base = ctx
        .config_get_toml("base_url")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    let api_key = ctx
        .config_get_toml("api_key")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    Ok(format!(
        "{}/core/api/jeeApi.php?apikey={api_key}&type=cmd&id={id}",
        base.trim_end_matches('/'),
    ))
}
```

(`read_value` becomes `let url = jeedom_url(ctx, sensor.id)?;` followed by the existing `match ctx.http_get_json(&url)`.)

Then the action machinery:

```rust
/// Executes one Jeedom ACTION command — same GET as a read; for an
/// action-type command the call runs it. The body (plain "ok", empty, or
/// JSON) is irrelevant: any 2xx counts as executed.
fn exec_cmd(ctx: &HostCtx, id: u64) -> Result<(), ()> {
    let url = jeedom_url(ctx, id)?;
    match ctx.http_get_json(&url) {
        Ok(_) => Ok(()),
        Err(e) => {
            ctx.log("warn", &format!("jeedom: action exec failed: {e}"));
            Err(())
        }
    }
}

/// Pending confirmation, stored in the skill's tmp KV. The tmp store has
/// no delete: "clear" = overwrite with an EMPTY payload (1 s TTL), and
/// every reader treats an empty payload as absent.
#[derive(Debug, serde::Serialize, Deserialize)]
struct Pending {
    cmd_id: u64,
    label: String,
}

const PENDING_KEY: &str = "pending_action";
const PENDING_TTL_SEC: u64 = 30;

fn action_label(d: &ActionDevice, on: bool, en: bool) -> String {
    match (on, en) {
        (true, false) => format!("allumer {}", d.name),
        (false, false) => format!("éteindre {}", d.name),
        (true, true) => format!("turn on {}", d.name),
        (false, true) => format!("turn off {}", d.name),
    }
}

fn load_pending(ctx: &HostCtx) -> Option<Pending> {
    let bytes = ctx.tmp_get(PENDING_KEY).ok().flatten()?;
    if bytes.is_empty() {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn clear_pending(ctx: &HostCtx) {
    let _ = ctx.tmp_set(PENDING_KEY, b"", 1);
}

fn done_or_error(executed: Result<(), ()>, en: bool) -> SkillResponse {
    match executed {
        Ok(()) => SkillResponse::speak(if en { "Done." } else { "C'est fait." }),
        Err(()) => SkillResponse::speak(if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        }),
    }
}
```

Then, in `handle`, after the existing `jeedom.read.`/`jeedom.read_all.` blocks and before the `asked` slot handling, add:

```rust
        // On/off device intents: the key riding in the intent name is the
        // device's on_id, for both directions.
        let turn = intent
            .name
            .strip_prefix("jeedom.turn_on.")
            .map(|k| (k, true))
            .or_else(|| intent.name.strip_prefix("jeedom.turn_off.").map(|k| (k, false)));
        if let Some((key, on)) = turn {
            let Some(device) = key
                .parse::<u64>()
                .ok()
                .and_then(|k| actions(ctx).iter().find(|d| d.on_id == k))
            else {
                return Ok(SkillResponse::speak(if en {
                    "sorry, I don't know that device"
                } else {
                    "désolé, je ne connais pas cet appareil"
                }));
            };
            let cmd_id = if on { device.on_id } else { device.off_id };
            if device.confirm {
                let label = action_label(device, on, en);
                let pending = Pending { cmd_id, label: label.clone() };
                if let Ok(bytes) = serde_json::to_vec(&pending) {
                    let _ = ctx.tmp_set(PENDING_KEY, &bytes, PENDING_TTL_SEC);
                }
                return Ok(SkillResponse::speak(if en {
                    format!("Confirm: {label}?")
                } else {
                    format!("Tu confirmes : {label} ?")
                }));
            }
            return Ok(done_or_error(exec_cmd(ctx, cmd_id), en));
        }

        if intent.name == "jeedom.confirm" {
            return Ok(match load_pending(ctx) {
                Some(p) => {
                    clear_pending(ctx);
                    done_or_error(exec_cmd(ctx, p.cmd_id), en)
                }
                None => SkillResponse::speak(if en {
                    "Nothing to confirm."
                } else {
                    "Rien à confirmer."
                }),
            });
        }
        if intent.name == "jeedom.cancel" {
            return Ok(match load_pending(ctx) {
                Some(_) => {
                    clear_pending(ctx);
                    SkillResponse::speak(if en { "Cancelled." } else { "Annulé." })
                }
                None => SkillResponse::speak(if en {
                    "Nothing to confirm."
                } else {
                    "Rien à confirmer."
                }),
            });
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --manifest-path skills-jeedom/Cargo.toml
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Jeedom skill: execute on/off actions with optional spoken confirmation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Admin discovery — pair on/off action commands

**Files:**
- Modify: `crates/athena-voice-admin/src/jeedom.rs`

**Interfaces:**
- Consumes: the existing `parse_fulldata` walk and `cmd_id` helper.
- Produces: `struct DiscoveredAction { on_id: u64, off_id: u64 }` (Serialize, Debug, PartialEq); `DiscoveredEquipment` gains `actions: Vec<DiscoveredAction>`; `fn pair_actions(cmds: &[ActionCmd]) -> Vec<DiscoveredAction>` with `struct ActionCmd { id: u64, name: String, generic: Option<String> }`. The discovery JSON per equipment now carries `"actions": [{"on_id":…,"off_id":…}]` — Task 6's UI consumes it.

- [ ] **Step 1: Write the failing tests**

`jeedom.rs` has no tests module — append one:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ac(id: u64, name: &str, generic: Option<&str>) -> ActionCmd {
        ActionCmd {
            id,
            name: name.into(),
            generic: generic.map(String::from),
        }
    }

    #[test]
    fn pairs_by_generic_type_first() {
        let cmds = vec![
            ac(124, "Bouton On", Some("LIGHT_ON")),
            ac(125, "Bouton Off", Some("LIGHT_OFF")),
            ac(200, "Refresh", Some("DONT_CARE")),
        ];
        assert_eq!(
            pair_actions(&cmds),
            vec![DiscoveredAction { on_id: 124, off_id: 125 }]
        );
    }

    #[test]
    fn pairs_by_french_and_english_names() {
        let cmds = vec![
            ac(7, "Allumer", None),
            ac(8, "Éteindre", None),
            ac(30, "On", None),
            ac(31, "Off", None),
            ac(90, "Rafraîchir", None),
        ];
        let pairs = pair_actions(&cmds);
        assert!(pairs.contains(&DiscoveredAction { on_id: 7, off_id: 8 }));
        assert!(pairs.contains(&DiscoveredAction { on_id: 30, off_id: 31 }));
        assert_eq!(pairs.len(), 2, "unpaired leftovers are ignored");
    }

    #[test]
    fn fulldata_carries_paired_actions_per_equipment() {
        let raw = serde_json::json!([{
            "name": "Salon",
            "eqLogics": [{
                "name": "Lampe",
                "cmds": [
                    {"id": 1, "type": "info", "subType": "numeric", "name": "Puissance"},
                    {"id": 124, "type": "action", "subType": "other", "name": "On"},
                    {"id": 125, "type": "action", "subType": "other", "name": "Off"}
                ]
            }]
        }]);
        let rooms = parse_fulldata(&raw).unwrap();
        let eq = &rooms[0].equipments[0];
        assert_eq!(eq.cmds.len(), 1, "info commands unchanged");
        assert_eq!(eq.actions, vec![DiscoveredAction { on_id: 124, off_id: 125 }]);
    }

    #[test]
    fn equipment_with_only_actions_still_surfaces() {
        // An on/off-only device has no info commands; it must not be
        // dropped by the "no cmds → skip equipment" guard.
        let raw = serde_json::json!([{
            "name": "Garage",
            "eqLogics": [{
                "name": "Portail",
                "cmds": [
                    {"id": 30, "type": "action", "subType": "other", "name": "Marche"},
                    {"id": 31, "type": "action", "subType": "other", "name": "Arrêt"}
                ]
            }]
        }]);
        let rooms = parse_fulldata(&raw).unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].equipments[0].actions.len(), 1);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p athena-voice-admin --lib jeedom`
Expected: FAIL to compile (`ActionCmd`, `pair_actions`, `DiscoveredAction` undefined).

- [ ] **Step 3: Implement**

Add next to `DiscoveredCmd`:

```rust
/// One paired on/off device discovered on an equipment.
#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct DiscoveredAction {
    pub(crate) on_id: u64,
    pub(crate) off_id: u64,
}

/// A raw action-type command as seen in fullData, before pairing.
pub(crate) struct ActionCmd {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) generic: Option<String>,
}

/// On/off name vocabulary, index-aligned: `NAME_ON[i]` pairs `NAME_OFF[i]`.
const NAME_ON: [&str; 4] = ["on", "allumer", "marche", "activer"];
const NAME_OFF: [&str; 4] = ["off", "éteindre", "arrêt", "désactiver"];

/// Pairs raw action commands into on/off devices: `generic_type`
/// (`FOO_ON`/`FOO_OFF` with the same prefix) first, then case-insensitive
/// name vocabulary. Unpaired commands are ignored (dimmers, scenarios and
/// friends are out of scope).
pub(crate) fn pair_actions(cmds: &[ActionCmd]) -> Vec<DiscoveredAction> {
    let mut out = Vec::new();
    let mut used: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // Pass 1: generic_type prefixes.
    for c in cmds {
        let Some(prefix) = c.generic.as_deref().and_then(|g| g.strip_suffix("_ON")) else {
            continue;
        };
        let off_generic = format!("{prefix}_OFF");
        if let Some(off) = cmds
            .iter()
            .find(|o| o.generic.as_deref() == Some(off_generic.as_str()) && !used.contains(&o.id))
        {
            if used.insert(c.id) && used.insert(off.id) {
                out.push(DiscoveredAction { on_id: c.id, off_id: off.id });
            }
        }
    }
    // Pass 2: name vocabulary, index-aligned (On↔Off, Allumer↔Éteindre, …).
    for (i, on_name) in NAME_ON.iter().enumerate() {
        let on = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == *on_name);
        let off = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == NAME_OFF[i]);
        if let (Some(on), Some(off)) = (on, off) {
            used.insert(on.id);
            used.insert(off.id);
            out.push(DiscoveredAction { on_id: on.id, off_id: off.id });
        }
    }
    out
}
```

In `DiscoveredEquipment` add the field:

```rust
    actions: Vec<DiscoveredAction>,
```

In `parse_fulldata`'s command loop, replace the early `type != "info" → continue` skip so action commands are collected (keep everything else identical):

```rust
            let mut action_cmds: Vec<ActionCmd> = Vec::new();
            for cmd in cmd_iter {
                let cmd_type = cmd.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let Some(id) = cmd.get("id").and_then(cmd_id) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                if cmd_type == "action" {
                    action_cmds.push(ActionCmd {
                        id,
                        name: name.to_string(),
                        generic: cmd
                            .get("generic_type")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                    });
                    continue;
                }
                if cmd_type != "info" {
                    continue;
                }
                // …existing DiscoveredCmd construction unchanged…
            }
            let actions = pair_actions(&action_cmds);
            if !cmds.is_empty() || !actions.is_empty() {
                equipments.push(DiscoveredEquipment {
                    name: eq_name,
                    cmds,
                    actions,
                });
            }
```

(The old guard was `if !cmds.is_empty()` — it must become the OR shown above so action-only devices survive.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p athena-voice-admin`
Expected: all PASS (new module + existing suite; the discovery-shape tests in `tests/api.rs`, if any assert on equipment JSON, may need the new `actions: []` key — fix them by asserting presence rather than exact shape).

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add crates/athena-voice-admin/src/jeedom.rs crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin discovery: pair Jeedom on/off action commands into devices

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Admin UI — action rows in the discovery tree and actions table

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js`

**Interfaces:**
- Consumes: `eq.actions` (`[{on_id, off_id}]`) from Task 5; the `actions` schema List field (renders automatically via `listEditor`) and its checkbox `confirm` cell from Tasks 2–3; existing helpers `composeSensorName`, `guessRoomPrefix`, `el`, `t`.
- Produces: discovery tree rows for paired devices that add rows `{name, on_id, off_id, room, prefix, confirm:false}` to the actions table; a "you can say" row detail on the actions table.

- [ ] **Step 1: Add i18n strings**

In both `T.en` and `T.fr` add:

```js
    // en
    action_onoff: 'on/off device',
    // fr
    action_onoff: 'appareil on/off',
```

- [ ] **Step 2: Thread the actions table through `renderDetail`**

Next to `findSensorsTable` (~line 393) add:

```js
  const findActionsTable = () =>
    widgets.find(([f]) => f.key === 'actions')?.[1].querySelector('table');
```

Give the actions table its own opts — after `sensorOpts` add:

```js
  const actionOpts = jd ? {
    onEdit: () => { jd.stale = true; findActionsTable()?.classList.add('stale'); },
    rowDetail: (row) => {
      const locales = jd.phraseGroups[`jeedom.turn_on.${Number(row.on_id)}`];
      const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
      if (!phrases.length) return null;
      return el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${phrases.slice(0, 2).map((p) => `« ${p} »`).join(', ')}` }));
    },
  } : undefined;
```

and use it in the `widgets` construction:

```js
    const w = fieldInput(f, skill.config[f.key],
      f.key === 'sensors' ? sensorOpts : f.key === 'actions' ? actionOpts : undefined);
```

Also make `refreshPhrases` clear the actions table's stale marker alongside the sensors table — after the existing `if (table) { … }` block add:

```js
    const atable = findActionsTable();
    if (atable) { atable.classList.remove('stale'); atable.rerender(); }
```

- [ ] **Step 3: Render action rows in the discovery tree**

Change `renderDiscoveryTree`'s signature and both call sites (the `discover` and `resync` button handlers) to pass the actions table:

```js
function renderDiscoveryTree(container, rooms, sensorsTable, actionsTable) {
```

call sites: `renderDiscoveryTree(tree, body.rooms, findSensorsTable(), findActionsTable());`

Inside the room/equipment loop, after the existing per-`cmd` rows, add per-device action rows:

```js
      for (const act of (eq.actions || [])) {
        const box = el('input', { type: 'checkbox' });
        const already = new Set((actionsTable?.getRows() || []).map((r) => Number(r.on_id)));
        box.checked = already.has(act.on_id);
        box.disabled = already.has(act.on_id);
        actionBoxes.push({ box, act, eqName: eq.name, room: room.name });
        section.append(el('div', { class: 'skill-row' },
          box,
          el('span', { class: 'name', text: `${eq.name} — on/off` }),
          el('span', { class: 'badge', text: t('action_onoff') }),
        ));
      }
```

with `const actionBoxes = [];` declared next to `const boxes = [];`, and extend the `add_selection` button's onclick to also push action rows:

```js
      const pickedActions = actionBoxes.filter(({ box }) => box.checked && !box.disabled);
      actionsTable?.addRows(pickedActions.map(({ act, eqName, room }) => ({
        name: composeSensorName('état', eqName, room),
        on_id: act.on_id,
        off_id: act.off_id,
        room: (room || '').toLowerCase(),
        prefix: guessRoomPrefix(room),
        confirm: false,
      })));
```

(`composeSensorName('état', eqName, room)` deliberately passes a generic command name so the composed spoken name comes from the EQUIPMENT name — "portail du garage" — exactly like generic sensor commands do.)

- [ ] **Step 4: Verify and rebuild**

Run: `node --check crates/athena-voice-admin/static/app.js && cargo test -p athena-voice-admin`
Expected: no output / PASS (assets are embedded at compile time; the test run rebuilds them).

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin UI: discovered on/off devices flow into the actions table

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: End-to-end phrase pin through wasm and registry

**Files:**
- Test: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: everything — the rebuilt `JEEDOM_TEST_WASM` fixture (runtime build.rs recompiles it from `skills-jeedom/src`), the `actions` config key from Task 3.
- Produces: a regression pin that a configured action device's phrase survives the wasm + registry round trip.

- [ ] **Step 1: Write the test**

Model it on `jeedom_phrases_lists_per_sensor_rules_for_every_locale` (same file, ~line 1038) — same fixture copy, registry load, and phrases call, with an `actions` config instead:

```rust
#[tokio::test]
async fn jeedom_phrases_include_action_devices() {
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        athena_voice_runtime::test_support::JEEDOM_TEST_WASM,
        skills_dir.path().join("jeedom.wasm"),
    )
    .expect("copy jeedom.wasm into the skills dir fixture");
    let per_skill = HashMap::from([(
        "jeedom".to_string(),
        SkillConfig {
            config: HashMap::from([(
                "actions".to_string(),
                r#"[{"name":"lumière du salon","room":"salon","prefix":"du",
                     "on_id":124,"off_id":125,"confirm":true}]"#
                    .to_string(),
            )]),
            ..Default::default()
        },
    )]);
    let load_deps = test_skill_deps_with(store.clone(), per_skill);
    let registry = SkillRegistry::load_dir(skills_dir.path(), &load_deps)
        .expect("load configured jeedom.wasm");
    let deps = admin_deps(
        store,
        Arc::new(registry),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let app = router(deps);

    let res = app
        .oneshot(get("/api/skills/jeedom/phrases"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    let entries = body["phrases"].as_array().unwrap();

    let on = entries
        .iter()
        .find(|e| e["intent"] == "jeedom.turn_on.124" && e["locale"] == "fr")
        .expect("turn_on rule listed for fr");
    assert!(
        on["phrases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "allume la lumière du salon"),
        "action phrase must survive the wasm + registry round trip: {on}"
    );
    assert!(
        entries.iter().any(|e| e["intent"] == "jeedom.confirm"),
        "confirm rule must exist when a device requires confirmation"
    );
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p athena-voice-admin --test api jeedom_phrases_include_action_devices`
Expected: PASS directly — all production code landed in Tasks 1–6; this is a pin, and its red state was Tasks 3–4's compile failures. If it FAILS, the wasm fixture didn't pick up the skill change: run `cargo clean -p athena-voice-runtime` and retest before debugging the skill.

- [ ] **Step 3: Full workspace verification**

Run: `cargo test --workspace && cargo fmt --check`
Expected: everything PASSES, formatting clean.

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin test: action-device phrase pinned through wasm and registry

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Manual verification (human, after all tasks)

Needs the real Jeedom on the GEEKOM:

1. `./update.sh` after CI publishes, open the admin UI → jeedom → Discover.
2. Paired on/off devices appear beside sensors; add one, adjust the spoken name, tick "confirm" on something consequential, Save.
3. Voice or test console: "allume <device>" → "C'est fait." and the device turns on; with confirm ticked → "Tu confirmes : … ?", then "oui" executes, "annule" drops, waiting 30 s expires the request.
