# Jeedom Shutters + Admin UI Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Voice control of Jeedom roller shutters (open/close/stop/position + "all shutters"), discovered and configured through the admin UI, plus a structure & polish pass on that UI.

**Architecture:** A new `Shutter` device type in the `skills-jeedom` wasm skill (config-driven pattern rules + intent handler, reusing the existing pending-confirmation KV machinery), a `pair_shutters()` discovery pass in the admin server that runs before on/off pairing, and admin UI changes: localized column labels, a sectioned Jeedom page, a shutters table, and a rewritten stylesheet.

**Tech Stack:** Rust (extism-pdk wasm skill, axum admin server), vanilla JS/CSS admin UI, wiremock + tokio tests.

**Spec:** `docs/superpowers/specs/2026-08-13-jeedom-shutters-design.md`

## Global Constraints

- Every commit uses the Gekkotron identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. Never write the user's real name anywhere.
- Code comments cite the generic assist protocol / Jeedom HTTP API only — never any private app's internals.
- `skills-jeedom` is its own workspace: run its tests with `cargo test --manifest-path skills-jeedom/Cargo.toml`. Admin tests run from the repo root: `cargo test -p athena-voice-admin`.
- Shutter intent keys always use the shutter's `up_id` (mirror of on/off's `on_id`).
- `stop_id` / `slider_id` use `0` = "not set" (`#[serde(default)] u64`): the admin UI's number cells write nothing for untouched cells, and Jeedom command ids start at 1.
- Stop and the all-shutters commands NEVER ask for confirmation; open/close/position respect the per-shutter `confirm` flag.
- Jeedom FLAP position convention: 0 = fermé/closed, 100 = ouvert/open.
- No JS framework, no build step, no external assets in the admin UI.

---

### Task 1: Skill — `Shutter` struct, parsing, config schema

**Files:**
- Modify: `skills-jeedom/src/lib.rs`

**Interfaces:**
- Produces: `struct Shutter { name, room, prefix: String, up_id, down_id: u64, stop_id, slider_id: u64 /* 0 = unset */, confirm: bool }`, `fn parse_shutters(raw: &str) -> Vec<Shutter>`, `fn shutters(ctx: &HostCtx) -> &'static [Shutter]`, and a `shutters` List field in `config_schema`. Later tasks call `shutters(ctx)` and read these exact field names.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `skills-jeedom/src/lib.rs`, next to `actions_parse_cleans_and_defaults`:

```rust
fn sh(name: &str, up_id: u64, down_id: u64) -> Shutter {
    Shutter {
        name: name.into(),
        room: String::new(),
        prefix: String::new(),
        up_id,
        down_id,
        stop_id: 0,
        slider_id: 0,
        confirm: false,
    }
}

#[test]
fn shutters_parse_cleans_and_defaults() {
    let raw = r#"[{"name":"volet 🪟 du salon","up_id":210,"down_id":211},
                  {"name":"volet de la chambre","room":"chambre","prefix":"de la",
                   "up_id":220,"down_id":221,"stop_id":222,"slider_id":223,"confirm":true}]"#;
    let v = parse_shutters(raw);
    assert_eq!(v[0].name, "volet du salon", "symbols stripped");
    assert_eq!(v[0].stop_id, 0, "stop_id defaults to 0 = unset");
    assert_eq!(v[0].slider_id, 0, "slider_id defaults to 0 = unset");
    assert!(!v[0].confirm);
    assert_eq!(v[1].stop_id, 222);
    assert_eq!(v[1].slider_id, 223);
    assert!(v[1].confirm);
    assert_eq!(parse_shutters("not json"), Vec::<Shutter>::new().as_slice());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml shutters_parse`
Expected: FAIL — `Shutter` / `parse_shutters` not defined.

- [ ] **Step 3: Implement** — below the `ActionDevice` block in `skills-jeedom/src/lib.rs`:

```rust
/// One roller shutter: up/down Jeedom action command ids behind a single
/// spoken name, with optional stop and position-slider commands (0 = not
/// configured — Jeedom ids start at 1, and the admin UI leaves untouched
/// number cells absent). Like sensors, `name` stores the FULL composed
/// spoken form ("volet du salon").
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Shutter {
    name: String,
    #[serde(default)]
    room: String,
    #[serde(default)]
    prefix: String,
    up_id: u64,
    down_id: u64,
    #[serde(default)]
    stop_id: u64,
    #[serde(default)]
    slider_id: u64,
    /// True → open/close/position ask "Tu confirmes : … ?" first.
    /// Stop is never gated: stopping a moving shutter must be immediate.
    #[serde(default)]
    confirm: bool,
}

static SHUTTERS: OnceCell<Vec<Shutter>> = OnceCell::new();

fn parse_shutters(raw: &str) -> Vec<Shutter> {
    let Ok(mut v) = serde_json::from_str::<Vec<Shutter>>(raw) else {
        return Vec::new();
    };
    for s in &mut v {
        s.name = clean_spoken(&s.name);
        s.room = clean_spoken(&s.room);
        s.prefix = clean_spoken(&s.prefix.replace('’', "'"));
    }
    v
}

fn shutters(ctx: &HostCtx) -> &'static [Shutter] {
    SHUTTERS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("shutters").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            parse_shutters(&raw)
        })
        .as_slice()
}
```

Then in `config_schema`, append a fourth `ConfigField` after the `actions` one:

```rust
ConfigField {
    key: "shutters".into(),
    label: "Shutters".into(),
    kind: FieldKind::List,
    required: false,
    help: "Roller shutters: spoken name → Jeedom up/down action ids; stop/slider optional".into(),
    default: String::new(),
    item_fields: vec![
        ItemField { key: "name".into(), kind: FieldKind::String, required: true },
        ItemField { key: "up_id".into(), kind: FieldKind::Number, required: true },
        ItemField { key: "down_id".into(), kind: FieldKind::Number, required: true },
        ItemField { key: "stop_id".into(), kind: FieldKind::Number, required: false },
        ItemField { key: "slider_id".into(), kind: FieldKind::Number, required: false },
        ItemField { key: "room".into(), kind: FieldKind::String, required: false },
        ItemField { key: "prefix".into(), kind: FieldKind::String, required: false },
        ItemField { key: "confirm".into(), kind: FieldKind::Bool, required: false },
    ],
},
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all tests PASS (new one included).

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: Shutter config type with parsing and schema"
```

---

### Task 2: Skill — extract shared confirm/cancel rules

**Files:**
- Modify: `skills-jeedom/src/lib.rs` (move the confirm-rule block out of `action_rules`, adjust `pattern_rules`, update tests)

**Interfaces:**
- Consumes: `shutters(ctx)` from Task 1.
- Produces: `fn confirm_rules(locale: &str) -> Vec<PatternRule>` (the `jeedom.confirm` / `jeedom.cancel` rules, unconditionally); `action_rules` no longer emits them; `JeedomSkill::pattern_rules` emits them when ANY action device OR shutter has `confirm: true`.

- [ ] **Step 1: Update the existing test and add the new one** — replace `confirm_rules_only_when_a_device_requires_confirmation` with:

```rust
#[test]
fn action_rules_carry_no_confirm_rules() {
    // Confirm/cancel are shared between on/off devices and shutters and are
    // registered once by pattern_rules — never by action_rules itself.
    let confirmed = vec![a("portail", 30, 31, true)];
    assert!(
        !action_rules("fr", &confirmed)
            .iter()
            .any(|r| r.intent == "jeedom.confirm" || r.intent == "jeedom.cancel")
    );
}

#[test]
fn confirm_rules_cover_both_locales() {
    let fr = confirm_rules("fr");
    let confirm = fr.iter().find(|r| r.intent == "jeedom.confirm").unwrap();
    assert!(confirm.phrases.contains(&"oui".to_string()));
    assert!(
        fr.iter()
            .any(|r| r.intent == "jeedom.cancel" && r.phrases.contains(&"annule".to_string()))
    );
    let en = confirm_rules("en");
    assert!(
        en.iter()
            .any(|r| r.intent == "jeedom.confirm" && r.phrases.contains(&"yes".to_string()))
    );
    assert!(confirm_rules("de").is_empty(), "unknown locale yields nothing");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml confirm_rules`
Expected: FAIL — `confirm_rules` not defined (and the old test no longer exists).

- [ ] **Step 3: Implement** — cut the `if devices.iter().any(|d| d.confirm) { … }` block from the end of `action_rules` and turn it into:

```rust
/// The shared spoken confirm/cancel rules — registered once by
/// `pattern_rules` when any on/off device or shutter has `confirm: true`.
fn confirm_rules(locale: &str) -> Vec<PatternRule> {
    let (yes, no): (Vec<String>, Vec<String>) = match locale {
        "fr" => (
            vec!["oui".into(), "confirme".into(), "c'est confirmé".into()],
            vec!["non".into(), "annule".into()],
        ),
        "en" => (
            vec!["yes".into(), "confirm".into()],
            vec!["no".into(), "cancel".into()],
        ),
        _ => return Vec::new(),
    };
    vec![
        PatternRule { intent: "jeedom.confirm".into(), phrases: yes, slots: Vec::new() },
        PatternRule { intent: "jeedom.cancel".into(), phrases: no, slots: Vec::new() },
    ]
}
```

And rewrite `JeedomSkill::pattern_rules`:

```rust
fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
    let ctx = HostCtx::for_testing();
    let mut rules = rules_for(locale, sensors(&ctx));
    rules.extend(action_rules(locale, actions(&ctx)));
    rules.extend(shutter_rules(locale, shutters(&ctx)));
    if actions(&ctx).iter().any(|d| d.confirm) || shutters(&ctx).iter().any(|s| s.confirm) {
        rules.extend(confirm_rules(locale));
    }
    rules
}
```

For this task to compile before Task 3, add a placeholder-free minimal `shutter_rules` that Task 3 will grow (it is real, final code for the empty case):

```rust
/// Match rules for configured shutters (grown in the shutter-rules task).
fn shutter_rules(locale: &str, list: &[Shutter]) -> Vec<PatternRule> {
    let _ = locale;
    if list.is_empty() {
        return Vec::new();
    }
    Vec::new()
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: confirm/cancel rules shared across device kinds"
```

---

### Task 3: Skill — shutter pattern rules

**Files:**
- Modify: `skills-jeedom/src/lib.rs` (flesh out `shutter_rules`)

**Interfaces:**
- Consumes: `Shutter` (Task 1), `shutter_rules` stub (Task 2).
- Produces: rules with intents `jeedom.shutter_open.{up_id}`, `jeedom.shutter_close.{up_id}`, `jeedom.shutter_stop.{up_id}` (only when `stop_id != 0`), `jeedom.shutter_pos.{up_id}` (only when `slider_id != 0`, one `position` Number slot), `jeedom.shutter_open_all`, `jeedom.shutter_close_all`. Task 4/5 handle exactly these names.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn shutter_rules_generate_open_close_phrases() {
    let list = vec![sh("volet du salon", 210, 211)];
    let rules = shutter_rules("fr", &list);
    let open = rules.iter().find(|r| r.intent == "jeedom.shutter_open.210").unwrap();
    assert!(open.phrases.contains(&"ouvre le volet du salon".to_string()), "got: {:?}", open.phrases);
    assert!(open.phrases.contains(&"monte le volet du salon".to_string()));
    let close = rules.iter().find(|r| r.intent == "jeedom.shutter_close.210").unwrap();
    assert!(close.phrases.contains(&"ferme le volet du salon".to_string()), "got: {:?}", close.phrases);
    assert!(close.phrases.contains(&"baisse le volet du salon".to_string()));

    let en = shutter_rules("en", &list);
    assert!(en.iter().any(|r| r.intent == "jeedom.shutter_open.210"
        && r.phrases.contains(&"open the volet du salon".to_string())));
}

#[test]
fn shutter_stop_and_pos_rules_require_their_ids() {
    let plain = vec![sh("volet du salon", 210, 211)];
    let rules = shutter_rules("fr", &plain);
    assert!(!rules.iter().any(|r| r.intent.starts_with("jeedom.shutter_stop.")));
    assert!(!rules.iter().any(|r| r.intent.starts_with("jeedom.shutter_pos.")));

    let mut full = sh("volet du salon", 210, 211);
    full.stop_id = 212;
    full.slider_id = 213;
    let rules = shutter_rules("fr", &[full]);
    let stop = rules.iter().find(|r| r.intent == "jeedom.shutter_stop.210").unwrap();
    assert!(stop.phrases.contains(&"stop le volet du salon".to_string()), "got: {:?}", stop.phrases);
    let pos = rules.iter().find(|r| r.intent == "jeedom.shutter_pos.210").unwrap();
    assert!(
        pos.phrases.contains(&"ouvre le volet du salon à {position}".to_string()),
        "got: {:?}", pos.phrases
    );
    assert_eq!(pos.slots.len(), 1);
    assert_eq!(pos.slots[0].name, "position");
    assert!(matches!(pos.slots[0].kind, SlotKind::Number));
}

#[test]
fn shutter_group_rules_exist_once() {
    let list = vec![sh("volet du salon", 210, 211), sh("volet de la chambre", 220, 221)];
    let rules = shutter_rules("fr", &list);
    let all_open: Vec<_> = rules.iter().filter(|r| r.intent == "jeedom.shutter_open_all").collect();
    assert_eq!(all_open.len(), 1, "one group rule regardless of shutter count");
    assert!(all_open[0].phrases.contains(&"ouvre tous les volets".to_string()));
    assert!(rules.iter().any(|r| r.intent == "jeedom.shutter_close_all"
        && r.phrases.contains(&"ferme tous les volets".to_string())));
    assert!(shutter_rules("fr", &[]).is_empty(), "no shutters, no rules");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml shutter_`
Expected: FAIL — rules are empty.

- [ ] **Step 3: Implement** — replace the Task 2 stub body:

```rust
/// Match rules for configured shutters. Stop and position rules exist only
/// for shutters that configured those command ids; the two group rules are
/// registered once whenever any shutter exists. Key = `up_id`.
fn shutter_rules(locale: &str, list: &[Shutter]) -> Vec<PatternRule> {
    if list.is_empty() {
        return Vec::new();
    }
    let mut rules = Vec::new();
    for s in list {
        let name = &s.name;
        let (open, close, stop, pos): (Vec<String>, Vec<String>, Vec<String>, Vec<String>) =
            match locale {
                "fr" => (
                    vec![
                        format!("ouvre le {name}"),
                        format!("ouvre la {name}"),
                        format!("ouvre {name}"),
                        format!("monte le {name}"),
                        format!("lève le {name}"),
                    ],
                    vec![
                        format!("ferme le {name}"),
                        format!("ferme la {name}"),
                        format!("ferme {name}"),
                        format!("descends le {name}"),
                        format!("baisse le {name}"),
                    ],
                    vec![
                        format!("stop le {name}"),
                        format!("stop {name}"),
                        format!("arrête le {name}"),
                    ],
                    vec![
                        format!("ouvre le {name} à {{position}}"),
                        format!("ouvre le {name} à {{position}} pour cent"),
                        format!("mets le {name} à {{position}}"),
                        format!("mets le {name} à {{position}} pour cent"),
                    ],
                ),
                "en" => (
                    vec![
                        format!("open the {name}"),
                        format!("open {name}"),
                        format!("raise the {name}"),
                    ],
                    vec![
                        format!("close the {name}"),
                        format!("close {name}"),
                        format!("lower the {name}"),
                    ],
                    vec![format!("stop the {name}"), format!("stop {name}")],
                    vec![
                        format!("set the {name} to {{position}}"),
                        format!("set the {name} to {{position}} percent"),
                        format!("open the {name} to {{position}} percent"),
                    ],
                ),
                _ => return Vec::new(),
            };
        rules.push(PatternRule {
            intent: format!("jeedom.shutter_open.{}", s.up_id),
            phrases: open,
            slots: Vec::new(),
        });
        rules.push(PatternRule {
            intent: format!("jeedom.shutter_close.{}", s.up_id),
            phrases: close,
            slots: Vec::new(),
        });
        if s.stop_id != 0 {
            rules.push(PatternRule {
                intent: format!("jeedom.shutter_stop.{}", s.up_id),
                phrases: stop,
                slots: Vec::new(),
            });
        }
        if s.slider_id != 0 {
            rules.push(PatternRule {
                intent: format!("jeedom.shutter_pos.{}", s.up_id),
                phrases: pos,
                slots: vec![SlotSpec { name: "position".into(), kind: SlotKind::Number }],
            });
        }
    }
    let (open_all, close_all): (Vec<String>, Vec<String>) = match locale {
        "fr" => (
            vec!["ouvre tous les volets".into(), "ouvre les volets".into()],
            vec!["ferme tous les volets".into(), "ferme les volets".into()],
        ),
        "en" => (
            vec!["open all the shutters".into(), "open the shutters".into()],
            vec!["close all the shutters".into(), "close the shutters".into()],
        ),
        _ => (Vec::new(), Vec::new()),
    };
    rules.push(PatternRule {
        intent: "jeedom.shutter_open_all".into(),
        phrases: open_all,
        slots: Vec::new(),
    });
    rules.push(PatternRule {
        intent: "jeedom.shutter_close_all".into(),
        phrases: close_all,
        slots: Vec::new(),
    });
    rules
}
```

Note: the `_ => return Vec::new()` inside the per-shutter loop matches `action_rules`' behavior for unknown locales; the group-rule match arm is unreachable for those locales but kept total.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: shutter open/close/stop/position/group pattern rules"
```

---

### Task 4: Skill — shutter intent handling (open/close/stop, confirm, position, group)

**Files:**
- Modify: `skills-jeedom/src/lib.rs` (handler arm in `JeedomSkill::handle`, `Pending`, new helpers)

**Interfaces:**
- Consumes: intent names from Task 3; existing `exec_cmd`, `done_or_error`, `Pending`, `load_pending`, `clear_pending`, `jeedom_url`.
- Produces: `Pending` gains `#[serde(default)] slider: Option<u64>`; `fn exec_slider(ctx: &HostCtx, id: u64, value: u64) -> Result<(), ()>`; `fn slot_number(v: &serde_json::Value) -> Option<f64>`; `fn shutter_label(name: &str, cmd: &ShutterCmd, en: bool) -> String`; `enum ShutterCmd { Open, Close, Stop, Pos(u64) }`; `fn group_answer(total: usize, failed: usize, en: bool) -> String`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn shutter_labels_phrase_both_locales() {
    assert_eq!(shutter_label("volet du salon", &ShutterCmd::Open, false), "ouvrir volet du salon");
    assert_eq!(shutter_label("volet du salon", &ShutterCmd::Close, false), "fermer volet du salon");
    assert_eq!(
        shutter_label("volet du salon", &ShutterCmd::Pos(50), false),
        "mettre volet du salon à 50 pour cent"
    );
    assert_eq!(shutter_label("volet du salon", &ShutterCmd::Open, true), "open volet du salon");
    assert_eq!(
        shutter_label("volet du salon", &ShutterCmd::Pos(50), true),
        "set volet du salon to 50 percent"
    );
}

#[test]
fn pending_slider_roundtrips_and_defaults() {
    // Old payloads (no slider field) must still load — the on/off flow
    // stores them and both flows share the same tmp key.
    let old = br#"{"cmd_id":124,"label":"allumer lampe"}"#;
    let p: Pending = serde_json::from_slice(old).unwrap();
    assert_eq!(p.slider, None);
    let new = Pending { cmd_id: 213, label: "mettre volet à 50 pour cent".into(), slider: Some(50) };
    let bytes = serde_json::to_vec(&new).unwrap();
    let back: Pending = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.slider, Some(50));
}

#[test]
fn slot_number_reads_strings_and_numbers() {
    assert_eq!(slot_number(&serde_json::json!("50")), Some(50.0));
    assert_eq!(slot_number(&serde_json::json!(" 50 ")), Some(50.0));
    assert_eq!(slot_number(&serde_json::json!(50)), Some(50.0));
    assert_eq!(slot_number(&serde_json::json!("cinquante")), None);
    assert_eq!(slot_number(&serde_json::Value::Null), None);
}

#[test]
fn group_answer_phrasing() {
    assert_eq!(group_answer(3, 0, false), "C'est fait.");
    assert_eq!(group_answer(3, 3, false), "désolé, je n'arrive pas à joindre Jeedom");
    assert_eq!(group_answer(3, 1, false), "C'est fait, mais un volet n'a pas répondu.");
    assert_eq!(group_answer(3, 2, false), "C'est fait, mais 2 volets n'ont pas répondu.");
    assert_eq!(group_answer(3, 1, true), "Done, but 1 shutter did not respond.");
    assert_eq!(group_answer(3, 2, true), "Done, but 2 shutters did not respond.");
}

#[test]
fn shutter_intent_for_unknown_device_apologises() {
    // Host-side config is empty, so no shutter matches key 999.
    let mut ctx = HostCtx::for_testing();
    let intent = Intent {
        name: "jeedom.shutter_open.999".into(),
        slots: Default::default(),
        locale: "fr".into(),
    };
    let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
    assert_eq!(speak_text(r), "désolé, je ne connais pas cet appareil");
}

#[test]
fn shutter_pos_without_number_reasks() {
    // No configured shutters → resolution fails before the slot is read,
    // so this exercises the unknown-device path; the re-ask path is only
    // reachable with config, which for_testing cannot supply. The re-ask
    // copy is pinned via ask_position() directly instead.
    assert_eq!(ask_position(false), "À quelle position, en pourcentage ?");
    assert_eq!(ask_position(true), "To what position, in percent?");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml shutter_ pending_slider slot_number group_answer`
(Just run the whole suite: `cargo test --manifest-path skills-jeedom/Cargo.toml`.)
Expected: FAIL — new types/functions missing.

- [ ] **Step 3: Implement.** First the small pieces near `Pending`/`action_label`:

```rust
/// What a shutter intent asks for; `Pos` carries the clamped 0–100 target.
/// `Copy` (payload is a bare u64) — the handler matches it by value twice.
#[derive(Debug, Clone, Copy)]
enum ShutterCmd {
    Open,
    Close,
    Stop,
    Pos(u64),
}

fn shutter_label(name: &str, cmd: &ShutterCmd, en: bool) -> String {
    match (cmd, en) {
        (ShutterCmd::Open, false) => format!("ouvrir {name}"),
        (ShutterCmd::Close, false) => format!("fermer {name}"),
        (ShutterCmd::Stop, false) => format!("arrêter {name}"),
        (ShutterCmd::Pos(v), false) => format!("mettre {name} à {v} pour cent"),
        (ShutterCmd::Open, true) => format!("open {name}"),
        (ShutterCmd::Close, true) => format!("close {name}"),
        (ShutterCmd::Stop, true) => format!("stop {name}"),
        (ShutterCmd::Pos(v), true) => format!("set {name} to {v} percent"),
    }
}

fn ask_position(en: bool) -> &'static str {
    if en {
        "To what position, in percent?"
    } else {
        "À quelle position, en pourcentage ?"
    }
}

/// The matcher inserts slot values as JSON strings; be tolerant of numbers.
fn slot_number(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn group_answer(total: usize, failed: usize, en: bool) -> String {
    if failed == 0 {
        return if en { "Done." } else { "C'est fait." }.into();
    }
    if failed >= total {
        return if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        }
        .into();
    }
    match (en, failed) {
        (false, 1) => "C'est fait, mais un volet n'a pas répondu.".into(),
        (false, n) => format!("C'est fait, mais {n} volets n'ont pas répondu."),
        (true, 1) => "Done, but 1 shutter did not respond.".into(),
        (true, n) => format!("Done, but {n} shutters did not respond."),
    }
}

/// Executes a Jeedom slider action command: same authenticated GET with the
/// target value as `&slider=`.
fn exec_slider(ctx: &HostCtx, id: u64, value: u64) -> Result<(), ()> {
    let url = jeedom_url(ctx, id)?;
    match ctx.http_get_json(&format!("{url}&slider={value}")) {
        Ok(_) => Ok(()),
        Err(e) => {
            ctx.log("warn", &format!("jeedom: slider exec failed: {e}"));
            Err(())
        }
    }
}
```

Extend `Pending` (field is serde-defaulted so stored on/off payloads keep loading):

```rust
#[derive(Debug, serde::Serialize, Deserialize)]
struct Pending {
    cmd_id: u64,
    label: String,
    /// Set for position commands: confirm executes cmd_id as a slider.
    #[serde(default)]
    slider: Option<u64>,
}
```

Update the two existing `Pending` construction sites: the on/off confirm branch gains `slider: None`, and `pending_roundtrips_through_json` gains `slider: None`. The `jeedom.confirm` handler branch changes its execution line to:

```rust
Some(p) => {
    clear_pending(ctx);
    let executed = match p.slider {
        Some(v) => exec_slider(ctx, p.cmd_id, v),
        None => exec_cmd(ctx, p.cmd_id),
    };
    done_or_error(executed, en)
}
```

Then the handler arm, inserted in `JeedomSkill::handle` right after the on/off `turn` block (before `jeedom.confirm`):

```rust
// Group shutter intents run every configured shutter, unconditionally
// (no confirmation: the command names its full scope already).
if intent.name == "jeedom.shutter_open_all" || intent.name == "jeedom.shutter_close_all" {
    let open = intent.name.ends_with("open_all");
    let list = shutters(ctx);
    if list.is_empty() {
        return Ok(SkillResponse::speak(if en {
            "sorry, I don't know that device"
        } else {
            "désolé, je ne connais pas cet appareil"
        }));
    }
    let failed = list
        .iter()
        .filter(|s| exec_cmd(ctx, if open { s.up_id } else { s.down_id }).is_err())
        .count();
    return Ok(SkillResponse::speak(group_answer(list.len(), failed, en)));
}

// Per-shutter intents: the key riding in the intent name is up_id.
let shutter_cmd = intent
    .name
    .strip_prefix("jeedom.shutter_open.")
    .map(|k| (k, ShutterCmd::Open))
    .or_else(|| intent.name.strip_prefix("jeedom.shutter_close.").map(|k| (k, ShutterCmd::Close)))
    .or_else(|| intent.name.strip_prefix("jeedom.shutter_stop.").map(|k| (k, ShutterCmd::Stop)))
    .or_else(|| {
        intent.name.strip_prefix("jeedom.shutter_pos.").map(|k| (k, ShutterCmd::Pos(0)))
    });
if let Some((key, mut cmd)) = shutter_cmd {
    let Some(shutter) = key
        .parse::<u64>()
        .ok()
        .and_then(|k| shutters(ctx).iter().find(|s| s.up_id == k))
    else {
        return Ok(SkillResponse::speak(if en {
            "sorry, I don't know that device"
        } else {
            "désolé, je ne connais pas cet appareil"
        }));
    };
    if let ShutterCmd::Pos(_) = cmd {
        let Some(p) = intent.slots.get("position").and_then(slot_number) else {
            return Ok(SkillResponse::speak(ask_position(en)));
        };
        cmd = ShutterCmd::Pos(p.clamp(0.0, 100.0) as u64);
    }
    let cmd_id = match cmd {
        ShutterCmd::Open => shutter.up_id,
        ShutterCmd::Close => shutter.down_id,
        ShutterCmd::Stop => shutter.stop_id,
        ShutterCmd::Pos(_) => shutter.slider_id,
    };
    if cmd_id == 0 {
        // Rules for stop/pos are only registered when the id is set, so
        // this is unreachable in practice — apologise defensively.
        return Ok(SkillResponse::speak(if en {
            "sorry, I don't know that device"
        } else {
            "désolé, je ne connais pas cet appareil"
        }));
    }
    // Stop is never gated behind confirmation.
    if shutter.confirm && !matches!(cmd, ShutterCmd::Stop) {
        let label = shutter_label(&shutter.name, &cmd, en);
        let slider = match cmd {
            ShutterCmd::Pos(v) => Some(v),
            _ => None,
        };
        let pending = Pending { cmd_id, label: label.clone(), slider };
        if let Ok(bytes) = serde_json::to_vec(&pending) {
            let _ = ctx.tmp_set(PENDING_KEY, &bytes, PENDING_TTL_SEC);
        }
        return Ok(SkillResponse::speak(if en {
            format!("Confirm: {label}?")
        } else {
            format!("Tu confirmes : {label} ?")
        }));
    }
    let executed = match cmd {
        ShutterCmd::Pos(v) => exec_slider(ctx, cmd_id, v),
        _ => exec_cmd(ctx, cmd_id),
    };
    return Ok(done_or_error(executed, en));
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS.

- [ ] **Step 5: Update the module doc header** — extend the `//!` block at the top of `skills-jeedom/src/lib.rs` with the `shutters` config key, mirroring the existing `sensors`/`actions` description (JSON list; `up_id`/`down_id` required; `stop_id`/`slider_id` optional with 0 = unset; `confirm` gates open/close/position but never stop; position sent as `&slider=N`, 0 = closed / 100 = open).

- [ ] **Step 6: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: execute shutter open/close/stop/position with confirm support"
```

---

### Task 5: Admin discovery — `pair_shutters` with precedence over on/off pairing

**Files:**
- Modify: `crates/athena-voice-admin/src/jeedom.rs`

**Interfaces:**
- Consumes: existing `ActionCmd`, `pair_actions`, `parse_fulldata`, `DiscoveredEquipment`.
- Produces: `ActionCmd` gains `pub(crate) subtype: String`; `pub(crate) struct DiscoveredShutter { up_id: u64, down_id: u64, stop_id: Option<u64>, slider_id: Option<u64> }` (serialized with `skip_serializing_if = "Option::is_none"` so the JSON omits absent ids); `pub(crate) fn pair_shutters(cmds: &[ActionCmd], used: &mut HashSet<u64>) -> Vec<DiscoveredShutter>`; `DiscoveredEquipment` gains `shutters: Vec<DiscoveredShutter>`. Task 6's fixture and Task 7's JS read `up_id`/`down_id`/`stop_id`/`slider_id` verbatim.

- [ ] **Step 1: Write the failing tests** — in the `tests` module of `crates/athena-voice-admin/src/jeedom.rs`, next to the `pair_actions` tests. The existing test helper for `ActionCmd` (look for how `pairs_by_generic_type_first` builds commands) must gain the `subtype` field; update all existing construction sites with `subtype: "other".into()`.

```rust
fn ac(id: u64, name: &str, generic: Option<&str>, subtype: &str) -> ActionCmd {
    ActionCmd {
        id,
        name: name.into(),
        generic: generic.map(String::from),
        subtype: subtype.into(),
    }
}

#[test]
fn shutters_pair_by_flap_generic_types() {
    let cmds = vec![
        ac(210, "Monter", Some("FLAP_UP"), "other"),
        ac(211, "Descendre", Some("FLAP_DOWN"), "other"),
        ac(212, "Stop", Some("FLAP_STOP"), "other"),
        ac(213, "Position", Some("FLAP_SLIDER"), "slider"),
    ];
    let mut used = std::collections::HashSet::new();
    let v = pair_shutters(&cmds, &mut used);
    assert_eq!(
        v,
        vec![DiscoveredShutter { up_id: 210, down_id: 211, stop_id: Some(212), slider_id: Some(213) }]
    );
    assert_eq!(used.len(), 4, "all four command ids consumed");
}

#[test]
fn shutters_pair_by_name_vocabulary() {
    let cmds = vec![
        ac(30, "Monter", None, "other"),
        ac(31, "Descendre", None, "other"),
        ac(32, "Stop", None, "other"),
    ];
    let mut used = std::collections::HashSet::new();
    let v = pair_shutters(&cmds, &mut used);
    assert_eq!(
        v,
        vec![DiscoveredShutter { up_id: 30, down_id: 31, stop_id: Some(32), slider_id: None }]
    );
}

#[test]
fn slider_attaches_by_subtype_without_generic() {
    let cmds = vec![
        ac(40, "Ouvrir", None, "other"),
        ac(41, "Fermer", None, "other"),
        ac(42, "Intensité", None, "slider"),
    ];
    let mut used = std::collections::HashSet::new();
    let v = pair_shutters(&cmds, &mut used);
    assert_eq!(v[0].slider_id, Some(42));
    assert_eq!(v[0].stop_id, None);
}

#[test]
fn shutter_pairing_leaves_onoff_commands_alone() {
    // A plug (On/Off) beside a shutter on the same equipment: the shutter
    // pass must consume only the FLAP commands so the on/off pass still
    // pairs the plug.
    let cmds = vec![
        ac(210, "Monter", Some("FLAP_UP"), "other"),
        ac(211, "Descendre", Some("FLAP_DOWN"), "other"),
        ac(50, "On", None, "other"),
        ac(51, "Off", None, "other"),
    ];
    let mut used = std::collections::HashSet::new();
    let shutters = pair_shutters(&cmds, &mut used);
    assert_eq!(shutters.len(), 1);
    let remaining: Vec<ActionCmd> = cmds.into_iter().filter(|c| !used.contains(&c.id)).collect();
    let actions = pair_actions(&remaining);
    assert_eq!(actions, vec![DiscoveredAction { on_id: 50, off_id: 51 }]);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p athena-voice-admin shutter`
Expected: FAIL — `DiscoveredShutter` / `pair_shutters` / `subtype` missing.

- [ ] **Step 3: Implement.** Add `subtype` to `ActionCmd`:

```rust
pub(crate) struct ActionCmd {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) generic: Option<String>,
    /// Jeedom `subType` ("other", "slider", …) — drives slider attachment.
    pub(crate) subtype: String,
}
```

In `parse_fulldata`, fill it where `ActionCmd` is built (the `cmd_type == "action"` branch already reads nothing for subtype; add):

```rust
subtype: cmd
    .get("subType")
    .and_then(|v| v.as_str())
    .unwrap_or("other")
    .to_string(),
```

Add the shutter types and pairing below `pair_actions`:

```rust
/// One paired shutter discovered on an equipment. Optional ids are omitted
/// from the JSON entirely (never null) so the client can copy fields
/// verbatim into config rows.
#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct DiscoveredShutter {
    pub(crate) up_id: u64,
    pub(crate) down_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stop_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slider_id: Option<u64>,
}

/// Shutter name vocabulary, index-aligned: `SHUTTER_UP[i]` pairs `SHUTTER_DOWN[i]`.
const SHUTTER_UP: [&str; 4] = ["monter", "monté", "up", "ouvrir"];
const SHUTTER_DOWN: [&str; 4] = ["descendre", "descendu", "down", "fermer"];
const SHUTTER_STOP_NAMES: [&str; 2] = ["stop", "arrêter"];

/// Pairs raw action commands into shutters: `FLAP_UP`/`FLAP_DOWN` generic
/// types first, then case-insensitive name vocabulary. A `FLAP_STOP` /
/// stop-named command attaches as stop; a `FLAP_SLIDER` / slider-subtype /
/// "position"-named command attaches as the position slider. Runs BEFORE
/// `pair_actions` — consumed ids land in `used` so the on/off pass cannot
/// claim them (an "Ouvrir"/"Fermer" pair is a shutter, not a switch).
pub(crate) fn pair_shutters(
    cmds: &[ActionCmd],
    used: &mut std::collections::HashSet<u64>,
) -> Vec<DiscoveredShutter> {
    let mut out = Vec::new();
    let mut push = |up: u64, down: u64, used: &mut std::collections::HashSet<u64>| {
        used.insert(up);
        used.insert(down);
        let stop_id = cmds
            .iter()
            .find(|c| {
                !used.contains(&c.id)
                    && (c.generic.as_deref() == Some("FLAP_STOP")
                        || SHUTTER_STOP_NAMES.contains(&c.name.to_lowercase().as_str()))
            })
            .map(|c| c.id);
        if let Some(id) = stop_id {
            used.insert(id);
        }
        let slider_id = cmds
            .iter()
            .find(|c| {
                !used.contains(&c.id)
                    && (c.generic.as_deref() == Some("FLAP_SLIDER")
                        || c.subtype == "slider"
                        || c.name.to_lowercase() == "position")
            })
            .map(|c| c.id);
        if let Some(id) = slider_id {
            used.insert(id);
        }
        out.push(DiscoveredShutter { up_id: up, down_id: down, stop_id, slider_id });
    };
    // Pass 1: FLAP generic types.
    for c in cmds {
        if used.contains(&c.id) || c.generic.as_deref() != Some("FLAP_UP") {
            continue;
        }
        if let Some(down) = cmds
            .iter()
            .find(|o| o.generic.as_deref() == Some("FLAP_DOWN") && !used.contains(&o.id))
        {
            push(c.id, down.id, used);
        }
    }
    // Pass 2: name vocabulary, index-aligned (Monter↔Descendre, …).
    for (i, up_name) in SHUTTER_UP.iter().enumerate() {
        let up = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == *up_name);
        let down = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == SHUTTER_DOWN[i]);
        if let (Some(up), Some(down)) = (up, down) {
            push(up.id, down.id, used);
        }
    }
    out
}
```

(If the closure borrowing `out`/`cmds` fights the borrow checker, make `push` a plain function taking `(cmds, up, down, used, &mut out)` — same body.)

Wire it into `parse_fulldata`: replace `let actions = pair_actions(&action_cmds);` with

```rust
let mut used = std::collections::HashSet::new();
let shutters = pair_shutters(&action_cmds, &mut used);
let remaining: Vec<ActionCmd> =
    action_cmds.into_iter().filter(|c| !used.contains(&c.id)).collect();
let actions = pair_actions(&remaining);
if !cmds.is_empty() || !actions.is_empty() || !shutters.is_empty() {
    equipments.push(DiscoveredEquipment { name: eq_name, cmds, actions, shutters });
}
```

and add `shutters: Vec<DiscoveredShutter>` to `DiscoveredEquipment`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p athena-voice-admin`
Expected: all PASS (existing pair_actions tests updated for the new `subtype` field).

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/src/jeedom.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin discovery: pair Jeedom FLAP commands into shutters before on/off"
```

---

### Task 6: Admin — discovery endpoint fixture + shutter phrases through the real wasm

**Files:**
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: `DiscoveredShutter` JSON shape (Task 5), shutter intents (Task 3) via the freshly built `JEEDOM_TEST_WASM` (the runtime's `build.rs` rebuilds it from `skills-jeedom` automatically).

- [ ] **Step 1: Extend `FULLDATA_FIXTURE`** with a third room (after "Garage"):

```json
  { "name": "Chambre", "eqLogics": [
    { "name": "Volet", "cmds": [
      { "id": 210, "name": "Monter", "type": "action", "subType": "other", "generic_type": "FLAP_UP" },
      { "id": 211, "name": "Descendre", "type": "action", "subType": "other", "generic_type": "FLAP_DOWN" },
      { "id": 212, "name": "Stop", "type": "action", "subType": "other", "generic_type": "FLAP_STOP" },
      { "id": 213, "name": "Position", "type": "action", "subType": "slider", "generic_type": "FLAP_SLIDER" }
    ] }
  ] }
```

Update `jeedom_discover_returns_info_command_tree`: `rooms.len()` becomes 3, and add:

```rust
let volet = &rooms[2]["equipments"][0];
assert_eq!(volet["cmds"].as_array().unwrap().len(), 0, "action cmds are not info cmds");
let shutters = volet["shutters"].as_array().unwrap();
assert_eq!(shutters.len(), 1);
assert_eq!(shutters[0]["up_id"], 210);
assert_eq!(shutters[0]["down_id"], 211);
assert_eq!(shutters[0]["stop_id"], 212);
assert_eq!(shutters[0]["slider_id"], 213);
```

Check whether other tests assert on `FULLDATA_FIXTURE` room counts (`jeedom_discover_prunes_empty_equipment_and_rooms` uses its own fixture — leave it) and adjust only what breaks.

- [ ] **Step 2: Add the wasm round-trip phrases test**, modeled exactly on `jeedom_phrases_include_action_devices`:

```rust
#[tokio::test]
async fn jeedom_phrases_include_shutters() {
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
                "shutters".to_string(),
                r#"[{"name":"volet de la chambre","room":"chambre","prefix":"de la",
                     "up_id":210,"down_id":211,"stop_id":212,"slider_id":213,"confirm":true}]"#
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

    let res = app.oneshot(get("/api/skills/jeedom/phrases")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    let entries = body["phrases"].as_array().unwrap();

    let open = entries
        .iter()
        .find(|e| e["intent"] == "jeedom.shutter_open.210" && e["locale"] == "fr")
        .expect("shutter_open rule listed for fr");
    assert!(
        open["phrases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "ouvre le volet de la chambre"),
        "shutter phrase must survive the wasm + registry round trip: {open}"
    );
    assert!(
        entries.iter().any(|e| e["intent"] == "jeedom.shutter_pos.210"),
        "position rule must exist when slider_id is set"
    );
    assert!(
        entries.iter().any(|e| e["intent"] == "jeedom.confirm"),
        "confirm rule must exist when a shutter requires confirmation"
    );
}
```

- [ ] **Step 3: Run to verify**

Run: `cargo test -p athena-voice-admin`
Expected: all PASS (build.rs recompiles the jeedom test wasm with Tasks 1–4's changes).

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin tests: shutter discovery fixture and wasm phrase round trip"
```

---

### Task 7: Admin UI — column labels, sections, shutters table, discovery rows

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js`

**Interfaces:**
- Consumes: discovery JSON `equipments[].shutters[] = { up_id, down_id, stop_id?, slider_id? }` (Task 5); phrase intents `jeedom.shutter_open.{up_id}` (Task 3); config schema field `shutters` (Task 1).
- Produces: browser-only behavior; no exported interfaces.

There is no JS test infrastructure — verification is the manual browser pass in Task 9. Keep each change small and re-check the page loads without console errors after each step (`cargo run -p athena-voice-cli -- serve` or the project's usual serve path, then open the admin UI).

- [ ] **Step 1: Localized strings + column labels.** Add to `T.en`:

```js
    action_shutter: 'shutter', with_stop: '+ stop', with_position: '+ position',
    section_connection: 'Connection', section_sensors: 'Sensors',
    section_actions: 'On/off devices', section_shutters: 'Shutters',
    section_discovery: 'Discovery',
```

and to `T.fr`:

```js
    action_shutter: 'volet', with_stop: '+ stop', with_position: '+ position',
    section_connection: 'Connexion', section_sensors: 'Capteurs',
    section_actions: 'Appareils on/off', section_shutters: 'Volets',
    section_discovery: 'Découverte',
```

Below the `T` block, add the column-label map and helper:

```js
// Human column headers for list editors; unknown keys fall back to the raw
// config key so non-jeedom skills stay readable.
const COL_LABELS = {
  en: {
    name: 'Spoken name', id: 'Cmd', unit: 'Unit', room: 'Room', prefix: 'Connector',
    kind: 'Type', on_label: 'ON label', off_label: 'OFF label',
    on_id: 'ON cmd', off_id: 'OFF cmd', confirm: 'Confirm',
    up_id: 'Up cmd', down_id: 'Down cmd', stop_id: 'Stop cmd', slider_id: 'Position cmd',
  },
  fr: {
    name: 'Nom parlé', id: 'Cmd', unit: 'Unité', room: 'Pièce', prefix: 'Liaison',
    kind: 'Type', on_label: 'Libellé ON', off_label: 'Libellé OFF',
    on_id: 'Cmd ON', off_id: 'Cmd OFF', confirm: 'Confirmation',
    up_id: 'Cmd monter', down_id: 'Cmd descendre', stop_id: 'Cmd stop', slider_id: 'Cmd position',
  },
};
const colLabel = (k) => COL_LABELS[lang][k] || k;
```

In `listEditor`, change the header row to use it: `el('th', { text: colLabel(c.key) })`.

- [ ] **Step 2: Shutters table wiring.** In `renderDetail`, next to `findActionsTable`:

```js
  const findShuttersTable = () =>
    widgets.find(([f]) => f.key === 'shutters')?.[1].querySelector('table');
```

Add `shutterOpts` after `actionOpts` (phrase hints keyed by up_id + the same prefix normalization sensors get):

```js
  const shutterOpts = jd ? {
    onEdit: () => { jd.stale = true; findShuttersTable()?.classList.add('stale'); },
    rowDetail: (row) => {
      const locales = jd.phraseGroups[`jeedom.shutter_open.${Number(row.up_id)}`];
      const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
      if (!phrases.length) return null;
      return el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${phrases.slice(0, 2).map((p) => `« ${p} »`).join(', ')}` }));
    },
    onCellChange: (key, row, oldValue) => {
      if (key !== 'prefix') return;
      const typed = String(row.prefix || '');
      row.prefix = typed.replace(/’/g, "'").trim();
      const swapped = swapNameSuffix(
        String(row.name || ''), String(oldValue || ''),
        row.prefix, String(row.room || ''),
      );
      const nameChanged = swapped !== row.name;
      if (nameChanged) row.name = swapped;
      if (nameChanged || row.prefix !== typed) findShuttersTable()?.rerender();
    },
  } : undefined;
```

Route it in the `widgets` map: `f.key === 'shutters' ? shutterOpts : …`. In `refreshPhrases`, clear the stale marker on the shutters table too (mirror the `atable` lines with `findShuttersTable()`). Extend `duplicatePhrases` to shutter and action families — change its intent filter regex to:

```js
    if (!/^jeedom\.(read|turn_on|shutter_open)\.\d+$/.test(intent)) continue;
```

- [ ] **Step 3: Sectioned Jeedom page.** In `renderDetail`, for jeedom only, group widgets under titled sections instead of a flat append. Replace the `widgets = fields.map(…)` block with:

```js
  const sections = {};
  const sectionFor = (key) =>
    key === 'sensors' ? 'sensors' : key === 'actions' ? 'actions'
      : key === 'shutters' ? 'shutters' : 'connection';
  const section = (key) => {
    if (!sections[key]) {
      sections[key] = el('div', { class: 'section' }, el('h3', { text: t(`section_${key}`) }));
    }
    return sections[key];
  };
  const widgets = fields.map((f) => {
    const w = fieldInput(f, skill.config[f.key],
      f.key === 'sensors' ? sensorOpts : f.key === 'actions' ? actionOpts
        : f.key === 'shutters' ? shutterOpts : undefined);
    if (jd) section(sectionFor(f.key)).append(w); else card.append(w);
    return [f, w];
  });
  if (jd) {
    for (const key of ['connection', 'sensors', 'actions', 'shutters']) {
      if (sections[key]) card.append(sections[key]);
    }
  }
```

Then move the jeedom buttons into sections: the `test_connection` button appends to `section('connection')`, and the discover/re-sync buttons plus `jmsg, pmsg, tree` append to `section('discovery')`, which is appended to the card after the shutters section. Non-jeedom skills keep the flat layout (the `else card.append(w)` branch).

- [ ] **Step 4: Discovery tree shutter rows + add-selection.** In `renderDiscoveryTree`, add a `shuttersTable` parameter (both call sites pass `findShuttersTable()`), and:

```js
  const existingShutters = new Set((shuttersTable?.getRows() || []).map((r) => Number(r.up_id)));
  const shutterBoxes = [];
```

Inside the equipment loop, after the actions loop:

```js
      for (const sh of (eq.shutters || [])) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existingShutters.has(sh.up_id);
        box.disabled = existingShutters.has(sh.up_id);
        shutterBoxes.push({ box, sh, eqName: eq.name, room: room.name });
        const extras = [
          sh.stop_id !== undefined ? t('with_stop') : '',
          sh.slider_id !== undefined ? t('with_position') : '',
        ].filter(Boolean).join(' ');
        section.append(el('div', { class: 'skill-row' },
          box,
          el('span', { class: 'name', text: `${eq.name}${extras ? ` (${extras})` : ''}` }),
          el('span', { class: 'badge type-shutter', text: t('action_shutter') }),
        ));
      }
```

And in the add-selection handler, after `pickedActions`:

```js
      const pickedShutters = shutterBoxes.filter(({ box }) => box.checked && !box.disabled);
      // Compose from the EQUIPMENT name (like generic sensors): equipment
      // "Volet" in room "Chambre" → "volet de la chambre". Absent stop/slider
      // ids stay undefined and are dropped by JSON.stringify on save.
      shuttersTable?.addRows(pickedShutters.map(({ sh, eqName, room }) => ({
        name: composeSensorName('état', eqName, room),
        up_id: sh.up_id,
        down_id: sh.down_id,
        stop_id: sh.stop_id,
        slider_id: sh.slider_id,
        room: (room || '').toLowerCase(),
        prefix: guessRoomPrefix(room),
        confirm: false,
      })));
```

Also give the on/off badge its color class while here: `el('span', { class: 'badge type-onoff', text: t('action_onoff') })`.

- [ ] **Step 5: Manual smoke check.** Serve the admin UI, open the jeedom detail page in the browser: sections render in order (Connexion / Capteurs / Appareils on/off / Volets / Découverte), column headers are localized, no console errors. Add a shutter row by hand, save, and confirm the row round-trips after reload.

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: shutters table, sectioned Jeedom page, human column labels"
```

---

### Task 8: Admin UI — visual polish

**Files:**
- Modify: `crates/athena-voice-admin/static/style.css` (full rewrite)
- Modify: `crates/athena-voice-admin/static/index.html` (one meta tag)

**Interfaces:** none (presentation only). Class names used by app.js (`card`, `skill-row`, `badge`, `chip`, `hint`, `stale`, `row-detail`, `read-ok`, `read-err`, `quiet`, `danger`, `error`, `notice`, `secret-set`, `help`, `test-*`, `section`, `type-onoff`, `type-shutter`) must keep working.

- [ ] **Step 1: Add the color-scheme meta** to `index.html` `<head>`:

```html
  <meta name="color-scheme" content="light dark">
```

- [ ] **Step 2: Replace `style.css`** with:

```css
:root {
  --bg: #f4f6f8; --card: #fff; --ink: #1c2733; --muted: #64748b;
  --accent: #2563eb; --accent-ink: #fff; --danger: #b91c1c; --ok: #15803d;
  --line: #e2e8f0; --row: #f1f5f9;
  --shadow: 0 1px 2px rgb(15 23 42 / .05), 0 1px 6px rgb(15 23 42 / .06);
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f151c; --card: #1a222c; --ink: #e8edf2; --muted: #94a6b8;
    --accent: #60a5fa; --accent-ink: #0b1220; --danger: #f87171; --ok: #4ade80;
    --line: #2a3542; --row: #212b37;
    --shadow: 0 1px 2px rgb(0 0 0 / .35);
  }
}
* { box-sizing: border-box; }
body { margin: 0; font: 15px/1.5 system-ui, sans-serif; background: var(--bg); color: var(--ink); }
header {
  display: flex; align-items: baseline; gap: 1rem; padding: .9rem 1.5rem;
  border-bottom: 1px solid var(--line); background: var(--card);
}
h1 { font-size: 1.05rem; margin: 0; letter-spacing: .01em; }
#status { color: var(--muted); font-size: .85rem; }
main { max-width: 1200px; margin: 1.5rem auto; padding: 0 1rem; display: grid; gap: 1.25rem; }

.card {
  background: var(--card); border: 1px solid var(--line); border-radius: 12px;
  padding: 1.25rem 1.5rem; box-shadow: var(--shadow);
}
.card h2 { margin: 0 0 .5rem; font-size: 1rem; }
.section { margin-top: 1.25rem; padding-top: 1rem; border-top: 1px solid var(--line); }
.section > h3 {
  margin: 0 0 .35rem; font-size: .78rem; font-weight: 700;
  text-transform: uppercase; letter-spacing: .07em; color: var(--muted);
}

.skill-row { display: flex; align-items: center; gap: .75rem; padding: .5rem 0; border-top: 1px solid var(--line); }
.skill-row:first-of-type { border-top: 0; }
.skill-row .name { font-weight: 600; flex: 1; cursor: pointer; }

.badge {
  font-size: .75rem; padding: .1rem .55rem; border-radius: 999px;
  border: 1px solid var(--line); color: var(--muted); white-space: nowrap;
}
.badge.ok { color: var(--ok); border-color: currentColor; }
.badge.off { color: var(--danger); border-color: currentColor; }
.badge.type-onoff { color: var(--accent); border-color: currentColor; }
.badge.type-shutter { color: var(--ok); border-color: currentColor; }

label { display: block; margin: .75rem 0 .25rem; font-weight: 600; font-size: .85rem; }
.help { color: var(--muted); font-size: .8rem; margin: .2rem 0 0; }
input[type=text], input[type=password], input[type=number] {
  width: 100%; padding: .45rem .6rem; border: 1px solid var(--line); border-radius: 8px;
  background: var(--bg); color: var(--ink); font: inherit; font-size: .9rem;
}
select {
  padding: .35rem .4rem; border: 1px solid var(--line); border-radius: 8px;
  background: var(--bg); color: var(--ink); font: inherit; font-size: .85rem;
}
input:disabled, select:disabled { opacity: .45; }
input:focus-visible, select:focus-visible, button:focus-visible {
  outline: 2px solid var(--accent); outline-offset: 1px;
}

button {
  padding: .45rem .95rem; border: 0; border-radius: 8px; font: inherit; font-size: .85rem;
  background: var(--accent); color: var(--accent-ink); cursor: pointer;
}
button:hover { filter: brightness(1.08); }
button:active { transform: translateY(1px); }
button:disabled { opacity: .5; cursor: default; transform: none; }
button.quiet { background: transparent; color: var(--accent); padding: .3rem .5rem; }
button.quiet:hover { filter: none; text-decoration: underline; }
button.danger { background: var(--danger); color: #fff; }

table { width: 100%; border-collapse: collapse; margin-top: .25rem; }
th {
  position: sticky; top: 0; z-index: 1; background: var(--card);
  text-align: left; font-size: .75rem; text-transform: uppercase;
  letter-spacing: .05em; color: var(--muted); font-weight: 600;
  padding: .3rem .5rem; border-bottom: 1px solid var(--line);
}
td { padding: .3rem .5rem; border-bottom: 1px solid var(--line); font-size: .85rem; }
tr:hover td { background: var(--row); }
tr.row-detail:hover td, tr.row-detail td { background: transparent; }
table input[type=text], table input[type=number] { padding: .3rem .45rem; font-size: .85rem; }
.row-detail td { border-bottom: 1px solid var(--line); padding: 0 .5rem .35rem; color: var(--muted); font-size: .78rem; }
td:has(+ .row-detail), tr:has(+ .row-detail) td { border-bottom: 0; }

.error { color: var(--danger); min-height: 1.2em; }
.notice { color: var(--ok); }
.secret-set { color: var(--muted); font-size: .8rem; }
.read-ok { color: var(--ok); font-size: .85rem; white-space: nowrap; }
.read-err { color: var(--danger); font-size: .85rem; }
.hint { color: var(--muted); }
.chip {
  display: inline-block; font-size: .72rem; padding: .05rem .45rem; margin-right: .35rem;
  border-radius: 999px; border: 1px solid var(--line);
}
.chip.warn { color: var(--danger); border-color: currentColor; }
.chip.sync { color: var(--accent); border-color: currentColor; }
.chip button, .hint button { padding: 0 .3rem; font-size: .72rem; }
table.stale .hint::before { content: '· '; color: var(--accent); font-weight: 700; }

/* Test console */
.test-row { display: flex; gap: .5rem; }
.test-row .test-input { flex: 1; }
.test-help, .test-answer { color: var(--muted); }
.test-answer { min-height: 1.2em; }
.test-history { margin-top: .5rem; display: flex; flex-wrap: wrap; gap: .35rem; }
.test-history-item { cursor: pointer; }
```

Design note: row highlighting is hover-based rather than zebra striping — the sensors table interleaves full-width `.row-detail` rows, which would break `nth-child` stripe parity. The `tr:has(+ .row-detail)` rule visually attaches a detail line to its row; `:has` is supported in all evergreen browsers, and its absence only costs a hairline.

- [ ] **Step 3: Manual browser pass** — light and dark (flip the OS theme): skills list, jeedom detail with all four sections, discovery tree, test console. Check hover/focus states, sticky table headers on a long sensors table, and that stale/duplicate/chip affordances still render.

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin/static/style.css crates/athena-voice-admin/static/index.html
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: refreshed visual skin (tables, buttons, sections, badges)"
```

---

### Task 9: Full verification + deployable wasm

**Files:**
- Modify: `skills/jeedom.wasm` (rebuilt artifact via `skills-jeedom/build.sh`, if that artifact is tracked — check `git status` after building; if untracked, skip committing it)

- [ ] **Step 1: Run every affected suite**

```bash
cargo test --manifest-path skills-jeedom/Cargo.toml
cargo test -p athena-voice-admin
cargo test -p athena-voice-runtime
```

Expected: all PASS. Fix anything that fails before proceeding (runtime tests exercise the rebuilt jeedom wasm through the registry).

- [ ] **Step 2: Rebuild the deployable skill wasm**

```bash
./skills-jeedom/build.sh
```

Expected: `✅ Copied to ../skills/jeedom.wasm`.

- [ ] **Step 3: End-to-end smoke via the admin test console** — serve locally, configure a fake shutter (`up_id`/`down_id` pointing at any reachable Jeedom or a mock), and type « ouvre le volet de la chambre » in the test console; expect « C'est fait. » (or the unreachable apology if no Jeedom — both prove routing).

- [ ] **Step 4: Final commit (if any artifacts/doc bits changed)**

```bash
git add -A
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom shutters: rebuilt skill wasm and final touches"
```

---

## Manual checklist (human, after merge)

- Real Jeedom on the GEEKOM: open/close/stop one shutter by voice; « mets le volet à 50 » ; « ferme tous les volets » ; confirm flow with the checkbox on.
- Browser pass over the restyled admin pages, light + dark.
