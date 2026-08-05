# Room Prefix Column Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-sensor French connector (« du », « d' », « chez »…) that drives the skill's room phrases and discovery's composed names, auto-filled by the existing article guess.

**Architecture:** Additive `prefix` field on the skill's `Sensor` (serde default → old configs unchanged) exposed through `config_schema.item_fields` so the admin table column appears schema-driven. `rules_for` branches on prefix: empty keeps the legacy both-genders enumeration verbatim; set generates de-form phrases plus locative « dans » forms only when the prefix maps to a definite article. UI extracts the article heuristic into `guessRoomPrefix`, fills prefix at discovery, swaps un-customized name suffixes on prefix edits, and re-sync composes fresh names with the row's stored prefix while never diffing prefix itself.

**Tech Stack:** Rust (skills-jeedom crate — host-side unit tests; wasm fixture auto-rebuilt by athena-voice-runtime's build.rs), vanilla JS.

**Spec:** `docs/superpowers/specs/2026-08-05-room-prefix-column-design.md` (approved).

## Global Constraints

- The `sensors` JSON change is additive only; configs without `prefix` must behave byte-identically to today (legacy phrase enumeration pinned by the existing `room_query_phrases_are_generated` test, which must pass unmodified).
- Straight apostrophes canonical: parse-time normalizes « ’ » → « ' » in prefix; spacing/elision checks accept both.
- « dans d'Alicia » must never be generated (unmapped prefixes get no dans-form).
- Prefix is NEVER part of re-sync diffs.
- No admin endpoint, assist-protocol, or satellite changes.
- Commit identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. Never write the owner's real name.
- Workspace lints `#![deny(warnings)]`; JS syntax gate is `node --check crates/athena-voice-admin/static/app.js`.
- If plain `cargo` fails on toolchain resolution locally, prefix commands with `RUSTUP_TOOLCHAIN=1.95`.

---

### Task 1: `Sensor.prefix` field, parse normalization, schema column

**Files:**
- Modify: `skills-jeedom/src/lib.rs` (Sensor struct ~line 36, `parse_sensors` ~line 85, `config_schema` ~line 446, tests helper `s()` ~line 522)

**Interfaces:**
- Produces: `Sensor.prefix: String` (serde default, cleaned + apostrophe-normalized at parse); `sensors.item_fields` gains `prefix` after `room` (so the admin column renders with zero UI change); test helper `sp(name, id, room, prefix)` used by Task 2's tests.

- [ ] **Step 1: Write the failing test**

In the `tests` module, after `parsing_strips_symbols_from_spoken_fields`:

```rust
    #[test]
    fn prefix_parses_cleans_and_defaults_empty() {
        let raw = r#"[{"name":"température d'alicia","id":7,"room":"alicia","prefix":"d’"},
                      {"name":"température du salon","id":8,"room":"salon"}]"#;
        let v = parse_sensors(raw);
        assert_eq!(
            v[0].prefix, "d'",
            "typographic apostrophe normalized to straight"
        );
        assert_eq!(v[1].prefix, "", "missing prefix defaults empty");
    }
```

And extend the `s()` helper so the struct literal still compiles once the field exists, plus the prefix-building helper Task 2 needs:

```rust
    fn s(name: &str, id: u64, unit: &str, room: &str, kind: &str, on: &str, off: &str) -> Sensor {
        Sensor {
            name: name.into(), id, unit: unit.into(), room: room.into(),
            kind: kind.into(), on_label: on.into(), off_label: off.into(),
            prefix: String::new(),
        }
    }

    fn sp(name: &str, id: u64, room: &str, prefix: &str) -> Sensor {
        Sensor {
            prefix: prefix.into(),
            ..s(name, id, "", room, "numeric", "", "")
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml prefix_parses_cleans_and_defaults_empty`
Expected: FAIL to compile — `Sensor` has no field `prefix`.

- [ ] **Step 3: Implement field + parse + schema**

In the `Sensor` struct, after `off_label`:

```rust
    /// French connector spoken before the room (« du », « de la », « d' »).
    /// Filled by discovery's article guess; empty keeps the legacy
    /// both-genders phrase enumeration.
    #[serde(default)]
    prefix: String,
```

In `parse_sensors`, alongside the other cleaned fields:

```rust
        s.prefix = clean_spoken(&s.prefix.replace('’', "'"));
```

In `config_schema`, insert after the `room` `ItemField`:

```rust
                    ItemField {
                        key: "prefix".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
```

And update the sensors field help string to: `"Spoken name → Jeedom command id; room/kind/prefix filled by discovery".into()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS (new test plus the untouched legacy suite).

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: optional per-sensor prefix field (parsed, cleaned, in schema)"
```

---

### Task 2: Prefix-driven phrase generation + metric stripping

**Files:**
- Modify: `skills-jeedom/src/lib.rs` (`rules_for` fr room branch ~line 346, `metric_of` ~line 257, new helpers above `rules_for`)

**Interfaces:**
- Consumes: `Sensor.prefix`, `sp()` from Task 1.
- Produces: `fn join_prefix(prefix: &str, room: &str) -> String` (elision-aware join) and `fn dans_article(prefix: &str) -> Option<&'static str>` (`du→le`, `de la→la`, `de l'/de l’→l'`, else None). Task 3's end-to-end assertion depends on the exact phrase « quelle est la température d'alicia ».

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn prefix_generates_elided_phrases_without_dans_forms() {
        let list = vec![sp("température d'alicia", 7, "alicia", "d'")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules
            .iter()
            .flat_map(|r| r.phrases.iter().map(String::as_str))
            .collect();
        assert!(all.contains(&"quelle est la température d'alicia"), "got: {all:?}");
        assert!(all.contains(&"température d'alicia"), "got: {all:?}");
        assert!(
            !all.iter().any(|p| p.contains("dans")),
            "no dans-form for an unmapped prefix: {all:?}"
        );
        assert!(
            !all.iter().any(|p| p.contains("du alicia") || p.contains("de la alicia")),
            "legacy article enumeration must be gone when a prefix is set: {all:?}"
        );
    }

    #[test]
    fn prefix_du_keeps_locative_dans_forms() {
        let list = vec![sp("température du salon", 142, "salon", "du")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules
            .iter()
            .flat_map(|r| r.phrases.iter().map(String::as_str))
            .collect();
        assert!(all.contains(&"température du salon"), "got: {all:?}");
        assert!(all.contains(&"quelle est la température du salon"), "got: {all:?}");
        assert!(all.contains(&"température dans le salon"), "got: {all:?}");
        assert!(all.contains(&"quelle température dans le salon"), "got: {all:?}");
        assert!(all.contains(&"quelle est la température dans le salon"), "got: {all:?}");
        assert!(
            !all.contains(&"température de la salon"),
            "wrong-gender enumeration gone when prefix set: {all:?}"
        );
    }

    #[test]
    fn metric_word_strips_configured_prefix() {
        assert_eq!(metric_of(&sp("température d'alicia", 7, "alicia", "d'")), "température");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml prefix_`
Expected: the two phrase tests FAIL (phrases like « du alicia » present, « d'alicia » absent); `metric_word_strips_configured_prefix` FAILS with « température d' ».

- [ ] **Step 3: Implement helpers + branches**

Above `rules_for`:

```rust
/// Joins a French connector to a room: no space after an elided form
/// (« d'alicia »), one space otherwise (« du salon »).
fn join_prefix(prefix: &str, room: &str) -> String {
    if prefix.ends_with('\'') || prefix.ends_with('’') {
        format!("{prefix}{room}")
    } else {
        format!("{prefix} {room}")
    }
}

/// Definite article implied by a de-form prefix, for locative « dans … »
/// phrases. Unmapped prefixes (« d' », « chez ») get no dans-form —
/// « dans d'Alicia » must never exist.
fn dans_article(prefix: &str) -> Option<&'static str> {
    match prefix.trim() {
        "du" => Some("le"),
        "de la" => Some("la"),
        "de l'" | "de l’" => Some("l'"),
        _ => None,
    }
}
```

In `rules_for`, replace ONLY the `"fr" => literal_phrases.extend([…])` arm of the room-scoped `match locale` with:

```rust
                "fr" => {
                    if sensor.prefix.is_empty() {
                        literal_phrases.extend([
                            format!("quelle {metric} dans le {room}"),
                            format!("quelle {metric} dans la {room}"),
                            format!("{metric} dans le {room}"),
                            format!("{metric} dans la {room}"),
                            // Natural full-sentence forms: "quelle
                            // est la température dans le salon" / "… du salon".
                            format!("quelle est la {metric} dans le {room}"),
                            format!("quelle est la {metric} dans la {room}"),
                            format!("quelle est la {metric} du {room}"),
                            format!("quelle est la {metric} de la {room}"),
                            format!("{metric} du {room}"),
                            format!("{metric} de la {room}"),
                        ]);
                    } else {
                        let with_room = join_prefix(&sensor.prefix, room);
                        literal_phrases.extend([
                            format!("{metric} {with_room}"),
                            format!("quelle est la {metric} {with_room}"),
                        ]);
                        if let Some(article) = dans_article(&sensor.prefix) {
                            let loc = join_prefix(article, room);
                            literal_phrases.extend([
                                format!("{metric} dans {loc}"),
                                format!("quelle {metric} dans {loc}"),
                                format!("quelle est la {metric} dans {loc}"),
                            ]);
                        }
                    }
                }
```

(The empty-prefix list is today's list verbatim — copy it from the existing code, do not retype from here if they ever diverge.)

In `metric_of`, replace the article-strip block with a prefix-aware version:

```rust
    let head = name[..name.len() - room.len()].trim_end();
    let prefix = sensor.prefix.trim().to_lowercase();
    let head = if !prefix.is_empty() && head.ends_with(&prefix) {
        head[..head.len() - prefix.len()].trim_end()
    } else {
        ["du", "de la", "de l’", "de l'", "de", "dans le", "dans la"]
            .iter()
            .find_map(|art| head.strip_suffix(art))
            .unwrap_or(head)
            .trim_end()
    };
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path skills-jeedom/Cargo.toml`
Expected: all PASS — including the UNTOUCHED `room_query_phrases_are_generated` (empty-prefix regression pin) and `metric_word_strips_room_suffix`.

- [ ] **Step 5: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: prefix-driven room phrases with elision and dans-form mapping"
```

---

### Task 3: End-to-end phrases assertion through the admin endpoint

**Files:**
- Modify: `crates/athena-voice-admin/tests/api.rs` (`jeedom_phrases_lists_per_sensor_rules_for_every_locale`)

**Interfaces:**
- Consumes: Tasks 1–2 (JEEDOM_TEST_WASM is rebuilt from `skills-jeedom` source by the runtime's build.rs, so the new field flows into the fixture automatically).

- [ ] **Step 1: Extend the existing test (it will fail until rebuilt-with-Tasks-1-2 — which is already the case, so it should pass immediately; the point is pinning the contract end-to-end)**

In `jeedom_phrases_lists_per_sensor_rules_for_every_locale`, change the sensors JSON to two sensors:

```rust
                r#"[{"name":"température salon","id":142,"unit":"°C","room":"salon"},
                    {"name":"température d'alicia","id":7,"room":"alicia","prefix":"d'"}]"#
```

And add, after the existing `en` assertion:

```rust
    let alicia = entries
        .iter()
        .find(|e| e["intent"] == "jeedom.read.7" && e["locale"] == "fr")
        .expect("fr group for the prefixed sensor");
    assert!(
        alicia["phrases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "quelle est la température d'alicia"),
        "elided prefix phrase must survive the wasm + registry round trip: {alicia}"
    );
```

- [ ] **Step 2: Run to verify it passes**

Run: `cargo test -p athena-voice-admin jeedom_phrases`
Expected: PASS (build.rs recompiles the fixture first — slow first run). If it FAILS with the phrase missing, the fixture didn't rebuild: `touch skills-jeedom/src/lib.rs` and rerun.

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin test: prefixed sensor phrase pinned through wasm and registry"
```

---

### Task 4: UI — `guessRoomPrefix`, prefix-aware name composition, discovery fill

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js` (`composeSensorName` + new helpers near it; `renderDiscoveryTree` add-selection mapping)

**Interfaces:**
- Produces (all top-level pure): `guessRoomPrefix(room) -> string` (straight-quote « de l' » for vowels, `FR_ROOM_ARTICLES` lookup, else `''`); `composeSensorName(cmdName, eqName, room, prefix)` — 4th param optional, defaults to the guess; `composedRoomSuffix(prefix, room)` and `swapNameSuffix(name, oldPrefix, newPrefix, room)` for Task 5. Discovery rows now carry `prefix`.

- [ ] **Step 1: Replace `composeSensorName` and add the helpers**

Replace the existing `composeSensorName` function with:

```js
// Article guess for a room, as the de-form connector stored in the prefix
// column. Straight apostrophes only — the skill's parse-time cleaning
// normalizes « ’ » to « ' », and generated names should start clean.
function guessRoomPrefix(room) {
  const r = (room || '').toLowerCase();
  if (!r) return '';
  if (/^[aeéèiouy]/.test(r)) return "de l'";
  return FR_ROOM_ARTICLES[r] || '';
}

// "d'alicia" / "du salon" / bare room when no prefix — the one spacing rule
// shared by name composition and the prefix-edit suffix swap.
function composedRoomSuffix(prefix, room) {
  if (!room) return '';
  if (!prefix) return room;
  return /['’]$/.test(prefix) ? `${prefix}${room}` : `${prefix} ${room}`;
}

function composeSensorName(cmdName, eqName, room, prefix) {
  const isGeneric = GENERIC_CMD_NAMES.includes(cmdName.toLowerCase());
  const cmd = (isGeneric ? eqName : cmdName).toLowerCase();
  if (!room) return cmd;
  const r = room.toLowerCase();
  const p = prefix !== undefined ? prefix : guessRoomPrefix(r);
  return `${cmd} ${composedRoomSuffix(p, r)}`;
}

// Prefix edit: recompose the name ONLY when it still ends with the old
// prefix+room suffix (never customized); a hand-edited name always wins.
function swapNameSuffix(name, oldPrefix, newPrefix, room) {
  const r = (room || '').toLowerCase();
  const oldSuffix = composedRoomSuffix(oldPrefix, r);
  if (!r || !oldSuffix || !name.endsWith(oldSuffix)) return name;
  return name.slice(0, name.length - oldSuffix.length) + composedRoomSuffix(newPrefix, r);
}
```

(The old `composeSensorName` article lookup moves into `guessRoomPrefix`; the typographic « de l’ » becomes straight-quoted. `FR_ROOM_ARTICLES` and `GENERIC_CMD_NAMES` stay as they are.)

- [ ] **Step 2: Fill prefix at discovery**

In `renderDiscoveryTree`'s add-selection `addRows` mapping, add `prefix` (and the name composition picks the same guess by default):

```js
      sensorsTable?.addRows(picked.map(({ cmd, eqName, room }) => ({
        name: composeSensorName(cmd.name, eqName, room),
        id: cmd.id,
        unit: cmd.unit || '',
        room: (room || '').toLowerCase(),
        prefix: guessRoomPrefix(room),
        kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
        on_label: cmd.on_label || '',
        off_label: cmd.off_label || '',
      })));
```

- [ ] **Step 3: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Then desk-check in node (extract the helpers with the same eval trick used before):
`guessRoomPrefix('salon')` → `'du'`; `guessRoomPrefix('alicia')` → `"de l'"`; `guessRoomPrefix('zzz')` → `''`;
`composeSensorName('Température', 'Capteur', 'Salon')` → `'température du salon'` (unchanged output);
`swapNameSuffix("température de l'alicia", "de l'", "d'", 'alicia')` → `"température d'alicia"`;
`swapNameSuffix('ma sonde perso', "de l'", "d'", 'alicia')` → `'ma sonde perso'` (customized name untouched);
`swapNameSuffix('température alicia', '', "d'", 'alicia')` → `"température d'alicia"` (empty old prefix swaps the bare-room suffix).

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: room prefix guess extracted; discovery fills the prefix column"
```

---

### Task 5: UI — name auto-recompose on prefix edit

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js` (`listEditor` input branch; jeedom `sensorOpts`)

**Interfaces:**
- Consumes: `swapNameSuffix` from Task 4; existing `sensorOpts`/`findSensorsTable`.
- Produces: `opts.onCellChange(colKey, row, oldValue)` hook on `listEditor`, fired on a cell's `change` event with the value snapshotted at `focus` (by `change` time, `oninput` has already mutated the row).

- [ ] **Step 1: Add the hook to `listEditor`'s input branch**

Replace the input-branch body inside `listEditor`:

```js
            cell = el('input', { type: c.type === 'number' ? 'number' : 'text' });
            cell.value = row[c.key] ?? '';
            let before;
            cell.onfocus = () => { before = row[c.key]; };
            cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; edited(); };
            cell.onchange = () => { if (opts.onCellChange) opts.onCellChange(c.key, row, before); };
```

- [ ] **Step 2: Wire the jeedom prefix swap**

In `sensorOpts`, after `onEdit`:

```js
    onCellChange: (key, row, oldValue) => {
      if (key !== 'prefix') return;
      const swapped = swapNameSuffix(
        String(row.name || ''), String(oldValue || ''),
        String(row.prefix || ''), String(row.room || ''),
      );
      if (swapped !== row.name) { row.name = swapped; findSensorsTable()?.rerender(); }
    },
```

- [ ] **Step 3: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Desk-check by reading: the swap only rerenders when the name actually changed (rerender on `change` fires after blur, so no focus loss mid-typing).

- [ ] **Step 4: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: prefix edits recompose un-customized sensor names"
```

---

### Task 6: UI — re-sync honors stored prefix, never diffs it

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js` (re-sync button handler)

**Interfaces:**
- Consumes: `composeSensorName(cmd, eq, room, prefix)` from Task 4.
- Produces: re-sync `fresh` map keeps raw parts; the compared name is composed per row with the row's stored prefix (fallback: the guess); `prefix` is absent from diff fields.

- [ ] **Step 1: Rework the re-sync handler's diff section**

Replace the `fresh` construction and row loop inside the re-sync `onclick`:

```js
          const fresh = new Map();
          for (const room of body.rooms) {
            for (const eq of room.equipments) {
              for (const cmd of eq.cmds) {
                fresh.set(cmd.id, {
                  cmdName: cmd.name, eqName: eq.name, roomName: room.name,
                  room: (room.name || '').toLowerCase(),
                  unit: cmd.unit || '',
                  kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
                  on_label: cmd.on_label || '',
                  off_label: cmd.off_label || '',
                });
              }
            }
          }
          jd.diffs = {};
          jd.missing = new Set();
          for (const row of table?.getRows() || []) {
            const id = Number(row.id);
            const disc = fresh.get(id);
            if (!disc) { jd.missing.add(id); continue; }
            // The compared name honors the ROW's stored prefix — the user's
            // correction, not a Jeedom fact — and prefix itself is never
            // diffed, so re-sync can't offer to revert « d' » to « de l' ».
            const wanted = {
              name: composeSensorName(disc.cmdName, disc.eqName, disc.roomName,
                row.prefix ? String(row.prefix) : undefined),
              room: disc.room, unit: disc.unit, kind: disc.kind,
              on_label: disc.on_label, off_label: disc.off_label,
            };
            jd.diffs[id] = Object.entries(wanted)
              .filter(([field, value]) => {
                // Stored kind may be '' — the skill treats that as numeric.
                const stored = field === 'kind' ? (row.kind || 'numeric') : String(row[field] ?? '');
                return stored !== String(value);
              })
              .map(([field, value]) => ({ field, value }));
          }
```

(The `renderDiscoveryTree(tree, body.rooms, table)` and `table?.rerender()` lines after it stay as they are.)

- [ ] **Step 2: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Desk-check in node with the diff logic extracted: a row `{id: 7, name: "température d'alicia", room: 'alicia', prefix: "d'", unit: '', kind: 'numeric'}` against a discovered Alicia/Capteur/Température numeric command produces NO name diff and NO prefix diff; the same row without a stored prefix gets a name chip offering `"température de l'alicia"` (the guess).

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: re-sync composes names with the stored prefix, never diffs it"
```

---

### Task 7: Workspace verification

- [ ] **Step 1: Full checks**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
node --check crates/athena-voice-admin/static/app.js
```

Expected: all clean (the FSEvents watcher test may flake under full-workspace load — re-run it in isolation before suspecting a regression). Fix anything new; commit fixups with the Gekkotron identity.

- [ ] **Step 2: Post-ship note**

Live check is the owner's manual step on the GEEKOM: set « d' » on Alicia's row, save, ask « quelle est la température d'Alicia ». The rebuilt jeedom wasm reaches the box through the existing bundled-skill refresh on image update — no repo artifact to commit.

---

## Out of scope (per spec)

- Auto-detecting person names beyond the dumb article heuristic.
- English-locale prefixes (en keeps "in the {room}").
- Re-sync chips for prefix.
