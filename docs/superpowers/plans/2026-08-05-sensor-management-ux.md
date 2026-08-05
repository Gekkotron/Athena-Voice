# Sensor Management UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the jeedom admin sensor table trustworthy: per-row live read test, re-sync diff chips against a changing Jeedom, the right widget per column, and "vous pouvez dire…" phrase hints with duplicate/symbol warnings.

**Architecture:** Two new host-side admin endpoints (`POST /api/skills/jeedom/read/{id}`, `GET /api/skills/jeedom/phrases`) following the existing test/discover pattern in `crates/athena-voice-admin/src/jeedom.rs` — the API key never reaches the browser. The phrases endpoint reads the registry's per-plugin rule cache via a new public `SkillRegistry::skill_rules` accessor. The UI work extends the generic `listEditor` in `static/app.js` with declarative column hooks (selects, conditional enabling, row actions, row detail lines) that the jeedom detail view wires up; re-sync reuses the existing `/discover` endpoint and diffs client-side.

**Tech Stack:** Rust (axum, wiremock for tests), vanilla JS (no framework, no JS test harness — repo convention), plain CSS.

**Spec:** `docs/superpowers/specs/2026-08-05-sensor-management-ux-design.md` (approved 2026-08-05).

## Global Constraints

- No changes to the skill, the assist protocol, or the satellite protocol.
- No changes to the config storage shape (the `sensors` JSON stays as-is).
- The API key must never appear in any response body, log line, or the browser. Every new endpoint test asserts the key is absent from the body (existing convention).
- All new UI copy exists in BOTH `en` and `fr` in the `T` table of `app.js`.
- Workspace lints: crates use `#![deny(warnings)]` — code must be warning-free.
- Commit identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …` on every commit. Never write the owner's real name.
- If plain `cargo` fails on toolchain resolution locally, prefix commands with `RUSTUP_TOOLCHAIN=1.95`.
- Building admin tests compiles wasm fixtures via the runtime's `build.rs` (`JEEDOM_TEST_WASM`); no manual setup step is needed, but first builds are slow.
- JS has no test harness: the syntax gate is `node --check crates/athena-voice-admin/static/app.js`, and helper functions must be small and pure enough to review by reading.

---

### Task 1: `SkillRegistry::skill_rules` accessor

**Files:**
- Modify: `crates/athena-voice-runtime/src/wasm/registry.rs` (accessor next to `config_schema` ~line 372; test in the existing `tests` module)

**Interfaces:**
- Consumes: the registry's existing private `plugin_rules: Mutex<HashMap<String, PluginRules>>` cache (populated by `install`, cleared by `remove`).
- Produces: `pub fn skill_rules(&self, name: &str, locale: &str) -> Option<Vec<HostPatternRule>>` — `None` when the skill isn't loaded, `Some(vec![])` when loaded but the locale has no rules. Task 3's phrases endpoint calls this. `HostPatternRule` (already public via `athena_voice_runtime::intent`) has fields `intent: String`, `phrases: Vec<String>`, `slots: Vec<HostSlotSpec>` and derives `Serialize` but NOT `PartialEq` — tests must assert on fields, not whole-value equality.

- [ ] **Step 1: Write the failing test**

In the `tests` module at the bottom of `registry.rs`, after `install_caches_config_schema`:

```rust
    #[test]
    fn skill_rules_exposes_cached_rules_per_locale() {
        let reg = SkillRegistry::new();
        let mock = simple_mock(&[("fr", vec![rule("hello", "bonjour")])]);
        reg.install("greeter", mock, &["fr".to_string()]).unwrap();

        let rules = reg.skill_rules("greeter", "fr").expect("loaded skill");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].intent, "hello");
        assert_eq!(rules[0].phrases, vec!["bonjour".to_string()]);
        // Loaded skill, locale it never contributed to → empty vec, not None.
        assert!(reg.skill_rules("greeter", "de").expect("still loaded").is_empty());
        // Unknown skill → None.
        assert!(reg.skill_rules("nope", "fr").is_none());
        // Removal clears the cache entry.
        reg.remove("greeter");
        assert!(reg.skill_rules("greeter", "fr").is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p athena-voice-runtime skill_rules_exposes_cached_rules_per_locale`
Expected: FAIL to compile with "no method named `skill_rules`".

- [ ] **Step 3: Implement the accessor**

In `impl SkillRegistry`, directly after `config_schema` (after line ~378):

```rust
    /// Raw pattern rules the named skill contributed for `locale`, from the
    /// per-plugin cache captured at install/reload time — the registry's own
    /// record of that skill's `pattern_rules(locale)` export, so no guest
    /// call happens here. `None` when the skill isn't loaded; `Some(vec![])`
    /// when it is loaded but contributed nothing for this locale.
    #[must_use]
    pub fn skill_rules(&self, name: &str, locale: &str) -> Option<Vec<HostPatternRule>> {
        self.plugin_rules
            .lock()
            .expect("plugin_rules lock poisoned")
            .get(name)
            .map(|per_locale| per_locale.get(locale).cloned().unwrap_or_default())
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p athena-voice-runtime skill_rules_exposes_cached_rules_per_locale`
Expected: PASS. Also run `cargo test -p athena-voice-runtime` to confirm no other registry test broke.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-runtime/src/wasm/registry.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Registry: public skill_rules(name, locale) accessor over the rule cache"
```

---

### Task 2: `POST /api/skills/jeedom/read/{id}` endpoint

**Files:**
- Modify: `crates/athena-voice-admin/src/jeedom.rs` (new handler after `discover`)
- Modify: `crates/athena-voice-admin/src/lib.rs` (route, after the `/skills/jeedom/discover` route ~line 72)
- Test: `crates/athena-voice-admin/tests/api.rs` (append after `jeedom_discover_prunes_empty_equipment_and_rooms`)

**Interfaces:**
- Consumes: existing `resolved_config(&AppState) -> Option<JeedomCfg>` and `status_json(&str)` in `jeedom.rs`; existing test helpers `deps_with_jeedom_config(base_url)`, `post(uri)`, `test_deps()`.
- Produces: `pub(crate) async fn read_one(State(state): State<AppState>, Path(id): Path<u64>) -> Response`. Responses: `{"status":"ok","value":"21.5"}` or `{"status":"unconfigured"|"unreachable"|"bad_response"}`. Non-numeric `{id}` → HTTP 400 (axum's `Path<u64>` rejection). Task 4's Lire button calls this.

- [ ] **Step 1: Write the failing tests**

Append to `crates/athena-voice-admin/tests/api.rs`:

```rust
// Jeedom single-sensor read endpoint tests

async fn read_body(res: axum::response::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("sekret-key-123"),
        "api key must never be echoed"
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn jeedom_read_normalizes_value_shapes() {
    // The same tolerance as the skill's read path: bare JSON scalar,
    // string-wrapped number, and `{"value": …}` envelope all normalize to
    // the same spoken string.
    for (raw_body, expected) in [
        ("21.5", "21.5"),
        (r#""21.5""#, "21.5"),
        (r#"{"value":21.5}"#, "21.5"),
    ] {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/core/api/jeeApi.php"))
            .and(wiremock::matchers::query_param("type", "cmd"))
            .and(wiremock::matchers::query_param("id", "142"))
            .and(wiremock::matchers::query_param("apikey", "sekret-key-123"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(raw_body))
            .mount(&server)
            .await;
        let deps = deps_with_jeedom_config(&server.uri()).await;
        let app = router(deps);
        let res = app
            .oneshot(post("/api/skills/jeedom/read/142"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = read_body(res).await;
        assert_eq!(body["status"], "ok", "raw body {raw_body}");
        assert_eq!(body["value"], expected, "raw body {raw_body}");
    }
}

#[tokio::test]
async fn jeedom_read_prose_error_is_bad_response() {
    // A bad key gets Jeedom's prose sentence (HTTP 200, not JSON). Decision
    // pinned by the spec: prose body = bad_response here — a bad key would
    // have already failed the test/discover flow.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("Clé API non valide"))
        .mount(&server)
        .await;
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/read/142"))
        .await
        .unwrap();
    assert_eq!(read_body(res).await["status"], "bad_response");
}

#[tokio::test]
async fn jeedom_read_unreachable_and_unconfigured() {
    let deps = deps_with_jeedom_config("http://127.0.0.1:1").await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/read/142"))
        .await
        .unwrap();
    assert_eq!(read_body(res).await["status"], "unreachable");

    let deps = test_deps().await; // no jeedom config at all
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/read/142"))
        .await
        .unwrap();
    assert_eq!(read_body(res).await["status"], "unconfigured");
}

#[tokio::test]
async fn jeedom_read_non_numeric_id_is_400() {
    let deps = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/read/abc"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin jeedom_read`
Expected: FAIL — all four tests hit a 404/405 (route doesn't exist yet), so status/body assertions fail.

- [ ] **Step 3: Implement handler + route**

Append to `crates/athena-voice-admin/src/jeedom.rs` (also add `Path` to the axum extract import at the top: `use axum::extract::{Path, State};`):

```rust
/// Host-side single-sensor read with the SAVED merged config — same pattern
/// as test/discover: the API key never travels to the browser, and the URL
/// (which embeds the key as a query param) is never logged.
pub(crate) async fn read_one(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=cmd&id={id}",
        cfg.base_url, cfg.api_key
    );
    let Ok(resp) = state
        .http
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    else {
        return status_json("unreachable");
    };
    if !resp.status().is_success() {
        return status_json("bad_response");
    }
    let Ok(body) = resp.text().await else {
        return status_json("bad_response");
    };
    // A bad key gets Jeedom's prose sentence (HTTP 200) — not valid JSON, so
    // it lands on bad_response; test/discover already classify authorization.
    let Ok(raw) = serde_json::from_str::<serde_json::Value>(&body) else {
        return status_json("bad_response");
    };
    // Same tolerance as the skill's read path: bare scalar, string-wrapped
    // number, or a `{"value": …}` envelope.
    let value = raw.get("value").cloned().unwrap_or(raw);
    let spoken = match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    Json(serde_json::json!({ "status": "ok", "value": spoken })).into_response()
}
```

In `crates/athena-voice-admin/src/lib.rs`, after the `/skills/jeedom/discover` route:

```rust
        .route(
            "/skills/jeedom/read/{id}",
            axum::routing::post(jeedom::read_one),
        )
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin jeedom_read`
Expected: 4 tests PASS. If `jeedom_read_non_numeric_id_is_400` fails because axum's `Path<u64>` rejection is not 400, check the actual status in the failure output — axum 0.8 returns 400 for path deserialization failures; if this workspace's axum differs, wrap the param as `Path<String>` and parse manually, returning `(StatusCode::BAD_REQUEST, Json(json!({"error": "id must be a number"})))` on parse failure.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/src/jeedom.rs crates/athena-voice-admin/src/lib.rs crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin: host-side single-sensor read endpoint (jeedom read/{id})"
```

---

### Task 3: `GET /api/skills/jeedom/phrases` endpoint

**Files:**
- Modify: `crates/athena-voice-admin/src/jeedom.rs` (new handler after `read_one`)
- Modify: `crates/athena-voice-admin/src/lib.rs` (route)
- Test: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: Task 1's `registry.skill_rules(name, locale)`; `state.skills` (`Option<SkillsHandle>`) whose `deps.locales: Vec<String>` lists every configured locale; test helpers `test_skill_deps` / `admin_deps` (this task adds a `per_skill`-aware variant).
- Produces: `pub(crate) async fn phrases(State(state): State<AppState>) -> Response` returning `{"phrases": [{"intent": "jeedom.read.142", "locale": "fr", "phrases": ["…"]}, …]}` — raw rules, one entry per (rule, locale); the UI groups them. Skill not loaded (or no runtime at all) → `{"phrases": []}`. Task 5's hints consume this.

- [ ] **Step 1: Write the failing tests**

Append to `crates/athena-voice-admin/tests/api.rs`. This needs a `per_skill`-aware deps builder — add it next to `test_skill_deps` and reuse the existing `admin_deps`:

```rust
/// Like `test_skill_deps` but with per-skill config, so a loaded skill's
/// `pattern_rules` export actually sees settings (e.g. jeedom sensors).
fn test_skill_deps_with(
    store: Arc<dyn Store>,
    per_skill: HashMap<String, SkillConfig>,
) -> SkillDeps {
    SkillDeps {
        per_skill,
        ..test_skill_deps(store)
    }
}

#[tokio::test]
async fn jeedom_phrases_lists_per_sensor_rules_for_every_locale() {
    // Real JEEDOM_TEST_WASM, loaded with one configured sensor: the endpoint
    // must surface that sensor's literal rules under jeedom.read.{id} for
    // both configured locales, straight from the registry's rule cache.
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
                "sensors".to_string(),
                r#"[{"name":"température salon","id":142,"unit":"°C","room":"salon"}]"#
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
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let entries = body["phrases"].as_array().unwrap();
    assert!(!entries.is_empty());

    let fr_group = entries
        .iter()
        .find(|e| e["intent"] == "jeedom.read.142" && e["locale"] == "fr")
        .expect("fr literal rule group for the configured sensor");
    let fr_phrases: Vec<&str> = fr_group["phrases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert!(
        fr_phrases.contains(&"quelle est la température salon"),
        "expected the name-derived literal phrase, got {fr_phrases:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| e["intent"] == "jeedom.read.142" && e["locale"] == "en"),
        "en locale group must be present too"
    );
}

#[tokio::test]
async fn jeedom_phrases_empty_when_skill_not_loaded() {
    // No skill runtime at all (skills: None) → empty phrases, same shape.
    let deps = test_deps().await;
    let app = router(deps);
    let res = app.oneshot(get("/api/skills/jeedom/phrases")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!({ "phrases": [] }));

    // Runtime present but jeedom not loaded → also empty.
    let store = test_store().await;
    let dir = tempfile::tempdir().unwrap();
    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let app = router(deps);
    let res = app.oneshot(get("/api/skills/jeedom/phrases")).await.unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!({ "phrases": [] }));
}
```

Note: the fixture sensor name deliberately contains no symbols — the skill's parse-time `clean_spoken` would strip them and the assertion is about phrase generation, not cleaning.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin jeedom_phrases`
Expected: FAIL — 404 from the missing route.

- [ ] **Step 3: Implement handler + route**

Append to `crates/athena-voice-admin/src/jeedom.rs`:

```rust
/// Every configured locale's pattern rules from the LOADED jeedom skill, via
/// the registry's rule cache — the UI shows the truth about what can be said
/// instead of re-deriving phrase templates in JS. Reflects the SAVED config:
/// a config save reloads the plugin, which repopulates the cache.
pub(crate) async fn phrases(State(state): State<AppState>) -> Response {
    let mut out: Vec<serde_json::Value> = Vec::new();
    if let Some(handle) = &state.skills {
        for locale in &handle.deps.locales {
            for rule in handle
                .registry
                .skill_rules("jeedom", locale)
                .unwrap_or_default()
            {
                out.push(serde_json::json!({
                    "intent": rule.intent,
                    "locale": locale,
                    "phrases": rule.phrases,
                }));
            }
        }
    }
    Json(serde_json::json!({ "phrases": out })).into_response()
}
```

In `crates/athena-voice-admin/src/lib.rs`, after the read route from Task 2:

```rust
        .route("/skills/jeedom/phrases", get(jeedom::phrases))
```

(`get` is already imported in `lib.rs`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin jeedom_phrases`
Expected: 2 tests PASS. Then run the whole crate: `cargo test -p athena-voice-admin` — all green.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/src/jeedom.rs crates/athena-voice-admin/src/lib.rs crates/athena-voice-admin/tests/api.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin: jeedom phrases endpoint surfaces loaded pattern rules per locale"
```

---

### Task 4: `listEditor` column hooks, kind `<select>`, label gating, Lire button

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js` (i18n table, `fieldInput`, `listEditor`, `renderDetail`)
- Modify: `crates/athena-voice-admin/static/style.css`

**Interfaces:**
- Consumes: Task 2's `POST /api/skills/jeedom/read/{id}`; existing `el`, `api`, `t`, `T` helpers.
- Produces: `listEditor(f, current, opts = {})` with hooks `opts.selects: {colKey: [choices]}`, `opts.enabledWhen: {colKey: (row) => bool}`, `opts.rowActions: (row, i) => [Node…]`, `opts.rowDetail: (row, i) => Node|null`, `opts.onEdit: () => void`; the returned table keeps `getRows()` / `addRows()` and gains `rerender()`. `fieldInput(f, current, opts)` forwards `opts`. Inside `renderDetail`, a `jd` state object (`{reads: {}, reading: false, phraseGroups: {}, duplicates: Set, expanded: Set, stale: bool, diffs: {}, missing: Set}`) exists for jeedom and is consumed by Tasks 5–6. Task 5 fills `phraseGroups`/`duplicates`; Task 6 fills `diffs`/`missing`.

- [ ] **Step 1: Add i18n strings**

In the `T` table in `app.js`, extend `en` and `fr` (this step also adds Task 5/6 strings so copy lands once):

```js
    // in T.en, after nothing_discovered:
    read: 'Read', resync: 'Re-sync', you_can_say: 'You can say…',
    duplicate_phrase: 'duplicate — another sensor answers the same phrase',
    matched_as: 'matched as', apply: 'Apply', gone_from_jeedom: 'gone from Jeedom',
    no_phrases: 'no phrases — save sensors first',
```

```js
    // in T.fr, after nothing_discovered:
    read: 'Lire', resync: 'Re-synchroniser', you_can_say: 'Vous pouvez dire…',
    duplicate_phrase: 'doublon — un autre capteur répond à la même phrase',
    matched_as: 'entendu comme', apply: 'Appliquer', gone_from_jeedom: 'disparu de Jeedom',
    no_phrases: 'aucune phrase — enregistrez des capteurs',
```

- [ ] **Step 2: Rewrite `listEditor` with column hooks and thread `opts` through `fieldInput`**

Replace the existing `listEditor` function entirely:

```js
function listEditor(f, current, opts = {}) {
  let rows = [];
  try { rows = current && current.kind === 'plain' ? JSON.parse(current.value) : []; } catch {}
  const table = el('table', { 'data-list': f.key });
  const edited = () => { if (opts.onEdit) opts.onEdit(); };
  const render = () => {
    const actionCols = opts.rowActions ? 1 : 0;
    table.replaceChildren(
      el('tr', {},
        ...f.item_fields.map((c) => el('th', { text: c.key })),
        ...(opts.rowActions ? [el('th')] : []),
        el('th')),
      ...rows.flatMap((row, i) => {
        const tds = f.item_fields.map((c) => {
          const choices = opts.selects && opts.selects[c.key];
          let cell;
          if (choices) {
            cell = el('select', {}, ...choices.map((v) => el('option', { value: v, text: v })));
            cell.value = choices.includes(row[c.key]) ? row[c.key] : choices[0];
            // A select change can re-enable/disable sibling cells — re-render.
            cell.onchange = () => { row[c.key] = cell.value; edited(); render(); };
          } else {
            cell = el('input', { type: c.type === 'number' ? 'number' : 'text' });
            cell.value = row[c.key] ?? '';
            cell.oninput = () => { row[c.key] = c.type === 'number' ? Number(cell.value) : cell.value; edited(); };
          }
          const enabled = opts.enabledWhen && opts.enabledWhen[c.key];
          if (enabled) cell.disabled = !enabled(row);
          return el('td', {}, cell);
        });
        if (opts.rowActions) tds.push(el('td', {}, ...opts.rowActions(row, i)));
        tds.push(el('td', {}, el('button', {
          class: 'quiet', text: t('remove'),
          onclick: () => { rows.splice(i, 1); edited(); render(); },
        })));
        const trs = [el('tr', {}, ...tds)];
        const detail = opts.rowDetail && opts.rowDetail(row, i);
        if (detail) {
          const td = el('td', {}, detail);
          td.setAttribute('colspan', String(f.item_fields.length + 1 + actionCols));
          trs.push(el('tr', { class: 'row-detail' }, td));
        }
        return trs;
      }),
    );
  };
  render();
  table.getRows = () => rows;
  table.addRows = (newRows) => { rows.push(...newRows); edited(); render(); };
  table.rerender = render;
  return el('div', {},
    el('label', { text: f.label || f.key }), table,
    el('button', { class: 'quiet', text: t('add_row'), onclick: () => { rows.push({}); edited(); render(); } }),
    f.help ? el('p', { class: 'help', text: f.help }) : '',
  );
}
```

And change `fieldInput`'s first line to forward opts:

```js
function fieldInput(f, current, opts) {
  if (f.type === 'list') return listEditor(f, current, opts);
```

(rest of `fieldInput` unchanged.)

- [ ] **Step 3: Wire the jeedom sensor table in `renderDetail`**

In `renderDetail`, BEFORE the `const widgets = fields.map(…)` line, insert the jeedom state and opts (the `sensorDetail` reference is filled in by Task 5 — for now it returns `null`):

```js
  // --- jeedom sensor-table state: live reads (Task 4), phrase hints
  // (Task 5), re-sync diffs (Task 6). One object so rowDetail/rowActions
  // closures and the buttons below share it. ---
  const jd = skill.name === 'jeedom' ? {
    reads: {},            // sensor id -> {status, value}
    reading: false,       // one in-flight read at a time
    phraseGroups: {},     // intent -> {locale -> [phrases]}
    duplicates: new Set(),
    expanded: new Set(),  // sensor ids with the full phrase list shown
    stale: false,         // edits since the last phrases fetch
    diffs: {},            // sensor id -> [{field, value}]
    missing: new Set(),   // sensor ids absent from the last re-sync
  } : null;
  const findSensorsTable = () =>
    widgets.find(([f]) => f.key === 'sensors')?.[1].querySelector('table');
  const readCell = (row) => {
    const id = Number(row.id);
    const r = jd.reads[id];
    const out = el('span', { class: r && r.status === 'ok' ? 'read-ok' : 'read-err' });
    if (r) out.textContent = r.status === 'ok'
      ? `${r.value}${row.unit ? ` ${row.unit}` : ''}`
      : t(`jeedom_${r.status}`);
    return [el('button', {
      class: 'quiet', text: t('read'),
      onclick: async () => {
        if (jd.reading || !Number.isFinite(id)) return;
        jd.reading = true;
        try {
          const res = await api(`/api/skills/jeedom/read/${id}`, { method: 'POST' });
          jd.reads[id] = res.ok ? await res.json() : { status: 'bad_response' };
        } finally { jd.reading = false; }
        findSensorsTable()?.rerender();
      },
    }), out];
  };
  const sensorOpts = jd ? {
    selects: { kind: ['numeric', 'binary'] },
    enabledWhen: {
      on_label: (row) => row.kind === 'binary',
      off_label: (row) => row.kind === 'binary',
    },
    rowActions: readCell,
    rowDetail: () => null, // Task 5 replaces this with sensorDetail
    onEdit: () => { jd.stale = true; findSensorsTable()?.classList.add('stale'); },
  } : undefined;
```

Then change the widgets line to pass opts only for the sensors list:

```js
  const widgets = fields.map((f) => {
    const w = fieldInput(f, skill.config[f.key], f.key === 'sensors' ? sensorOpts : undefined);
    card.append(w);
    return [f, w];
  });
```

And DELETE the now-duplicate `findSensorsTable` const that currently sits inside the `if (skill.name === 'jeedom')` block (it moved above).

Note there's a forward-reference trap: `findSensorsTable` closes over `widgets`, which is declared after it. That's fine — it's only *called* on user clicks, long after `widgets` is initialized. Keep the `const widgets` declaration AFTER the block above.

- [ ] **Step 4: CSS for selects, read results, detail rows, chips**

Append to `style.css` (chips/detail/stale are used by Tasks 5–6 — landing the styles once here):

```css
select { padding: .35rem .4rem; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); color: var(--ink); }
input:disabled, select:disabled { opacity: .45; }
.read-ok { color: var(--ok); font-size: .85rem; white-space: nowrap; }
.read-err { color: var(--danger); font-size: .85rem; }
.row-detail td { border-top: 0; padding: 0 .4rem .3rem; color: var(--muted); font-size: .78rem; }
.hint { color: var(--muted); }
.chip { display: inline-block; font-size: .72rem; padding: .05rem .45rem; margin-right: .35rem; border-radius: 999px; border: 1px solid var(--line); }
.chip.warn { color: var(--danger); border-color: var(--danger); }
.chip.sync { color: var(--accent); border-color: var(--accent); }
.chip button, .hint button { padding: 0 .3rem; font-size: .72rem; }
table.stale .hint::before { content: '· '; color: var(--accent); font-weight: 700; }
```

- [ ] **Step 5: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Expected: no output (syntax OK).
Run: `cargo test -p athena-voice-admin static_assets_served_with_mime`
Expected: PASS (assets still embed and serve).

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-admin/static/app.js crates/athena-voice-admin/static/style.css
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: sensor table kind select, gated labels, per-row live read"
```

---

### Task 5: "Vous pouvez dire…" phrase hints with duplicate/symbol chips

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js`

**Interfaces:**
- Consumes: Task 3's `GET /api/skills/jeedom/phrases`; Task 4's `jd` state, `sensorOpts.rowDetail` slot, `table.rerender()`, `.stale` CSS, i18n strings.
- Produces: top-level pure helpers `normalizeLiteral(s)` and `duplicatePhrases(groups)`; `refreshPhrases()` inside `renderDetail`; `sensorDetail(row)` rendering the hint line + chips, wired as `rowDetail`. Task 6 appends its chips inside `sensorDetail` too (it reads `jd.diffs`/`jd.missing`, already rendered here as empty).

- [ ] **Step 1: Add the pure helpers**

In `app.js`, right after `composeSensorName` (top-level, so they're reviewable in isolation):

```js
// Mirror of the matcher's `normalize_literal` (runtime intent/engine.rs):
// lowercase; keep letters/digits of any script, apostrophes, and hyphens;
// collapse every dropped char or whitespace run into a single space. This is
// the form the matcher actually compares against speech, so it drives both
// the symbols chip ("entendu comme …") and duplicate detection.
function normalizeLiteral(s) {
  let out = '';
  let pending = false;
  for (const c of String(s).toLowerCase()) {
    if (/[\p{L}\p{N}'-]/u.test(c)) {
      if (pending && out) out += ' ';
      out += c;
      pending = false;
    } else {
      pending = true;
    }
  }
  return out;
}

// Normalized phrases that appear under MORE THAN ONE per-sensor intent
// (jeedom.read.{id}) — name collisions like six sensors all called
// "température". groups: intent -> {locale -> [phrases]}.
function duplicatePhrases(groups) {
  const firstIntent = new Map();
  const dupes = new Set();
  for (const [intent, locales] of Object.entries(groups)) {
    if (!/^jeedom\.read\.\d+$/.test(intent)) continue;
    for (const phrases of Object.values(locales)) {
      for (const p of phrases) {
        const n = normalizeLiteral(p);
        if (!firstIntent.has(n)) firstIntent.set(n, intent);
        else if (firstIntent.get(n) !== intent) dupes.add(n);
      }
    }
  }
  return dupes;
}
```

- [ ] **Step 2: Add `refreshPhrases` and `sensorDetail` in `renderDetail`**

Right after the `sensorOpts` definition from Task 4 (still before `const widgets = …`), add:

```js
  const pmsg = el('p', { class: 'help' });
  async function refreshPhrases() {
    if (!jd) return;
    let body;
    try { body = await (await api('/api/skills/jeedom/phrases')).json(); } catch { return; }
    jd.phraseGroups = {};
    for (const p of body.phrases) {
      (jd.phraseGroups[p.intent] ??= {})[p.locale] = p.phrases;
    }
    jd.duplicates = duplicatePhrases(jd.phraseGroups);
    jd.stale = false;
    const table = findSensorsTable();
    if (table) { table.classList.remove('stale'); table.rerender(); }
    const anySensorGroup = Object.keys(jd.phraseGroups).some((k) => /^jeedom\.read\.\d+$/.test(k));
    pmsg.textContent = anySensorGroup ? '' : t('no_phrases');
  }
  function sensorDetail(row) {
    const id = Number(row.id);
    const bits = [];
    // Re-sync outcome (filled by the Re-sync button): per-field apply chips
    // and the gone-from-Jeedom badge.
    if (jd.missing.has(id)) bits.push(el('span', { class: 'chip warn', text: t('gone_from_jeedom') }));
    for (const d of jd.diffs[id] || []) {
      bits.push(el('span', { class: 'chip sync' },
        el('span', { text: `Jeedom: ${d.field} = ${d.value === '' ? '—' : d.value} ` }),
        el('button', {
          class: 'quiet', text: t('apply'),
          onclick: () => {
            row[d.field] = d.value;
            jd.diffs[id] = (jd.diffs[id] || []).filter((x) => x !== d);
            jd.stale = true;
            const table = findSensorsTable();
            if (table) { table.classList.add('stale'); table.rerender(); }
          },
        }),
      ));
    }
    // Symbols chip: the stored name/room carries characters the matcher
    // strips — show the form actually compared against speech.
    for (const key of ['name', 'room']) {
      const v = String(row[key] || '');
      if (v && normalizeLiteral(v) !== v.toLowerCase()) {
        bits.push(el('span', { class: 'chip warn', text: `${t('matched_as')} « ${normalizeLiteral(v)} »` }));
        break;
      }
    }
    // "Vous pouvez dire…" — what the SAVED config generates for this sensor,
    // in the UI's language (fall back to any locale that has phrases).
    const locales = jd.phraseGroups[`jeedom.read.${id}`];
    const phrases = locales ? (locales[lang] || Object.values(locales)[0] || []) : [];
    if (phrases.length) {
      const shown = jd.expanded.has(id) ? phrases : phrases.slice(0, 2);
      const line = el('span', { class: 'hint' },
        el('span', { text: `${t('you_can_say')} ${shown.map((p) => `« ${p} »`).join(', ')}` }),
      );
      if (phrases.length > shown.length) {
        line.append(el('button', {
          class: 'quiet', text: ` +${phrases.length - shown.length}`,
          onclick: () => { jd.expanded.add(id); findSensorsTable()?.rerender(); },
        }));
      }
      if (phrases.some((p) => jd.duplicates.has(normalizeLiteral(p)))) {
        bits.push(el('span', { class: 'chip warn', text: t('duplicate_phrase') }));
      }
      bits.push(line);
    }
    if (!bits.length) return null;
    return el('span', {}, ...bits);
  }
```

Then update `sensorOpts.rowDetail` (from Task 4) to use it:

```js
    rowDetail: (row) => sensorDetail(row),
```

(JS function declarations hoist, so `sensorDetail` being defined after `sensorOpts` is fine; `refreshPhrases` is only called after everything exists.)

- [ ] **Step 3: Wire fetch-on-load and fetch-after-save**

In the `if (skill.name === 'jeedom')` block of `renderDetail`, append `pmsg` alongside `jmsg, tree` (i.e. `…, jmsg, pmsg, tree,`) and add an initial fetch right after that block:

```js
  if (jd) refreshPhrases();
```

In the save button's `onclick`, after the success/failure message assignment (`msg.className = …`), add:

```js
      if (jd && res.ok) refreshPhrases();
```

- [ ] **Step 4: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Expected: no output.
Desk-check the pure helpers against the Rust originals: `normalizeLiteral("Salon 🖴")` → `"salon"`, `normalizeLiteral("t°")` → `"t"`, `normalizeLiteral("l'étagère-basse")` → `"l'étagère-basse"` (apostrophe + hyphen survive), and `duplicatePhrases` with two intents sharing `"quelle est la température"` returns a 1-element set while the same phrase under one intent in two locales returns an empty set.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: per-sensor phrase hints with duplicate and symbol warnings"
```

---

### Task 6: Re-sync button with per-field apply chips

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js` (jeedom buttons block in `renderDetail`)

**Interfaces:**
- Consumes: existing `POST /api/skills/jeedom/discover` (NO new endpoint — spec decision), `composeSensorName`, `renderDiscoveryTree`, `jd.diffs`/`jd.missing` (rendered by Task 5's `sensorDetail`), `findSensorsTable`, i18n strings from Task 4.
- Produces: a "Re-synchroniser" button next to "Découvrir les capteurs". After discovery: per existing row (matched by id) one `{field, value}` diff per changed field among `name/room/unit/kind/on_label/off_label`; ids absent from discovery land in `jd.missing`; newly discovered sensors go through the unchanged `renderDiscoveryTree` flow.

- [ ] **Step 1: Add the Re-sync button**

In `renderDetail`'s `if (skill.name === 'jeedom')` block, after the existing discover button and before `jmsg, pmsg, tree`, add:

```js
      el('button', {
        class: 'quiet', text: t('resync'),
        onclick: async () => {
          jmsg.textContent = t('discovering');
          const body = await (await api('/api/skills/jeedom/discover', { method: 'POST' })).json();
          if (body.status !== 'ok') { jmsg.textContent = t(`jeedom_${body.status}`); return; }
          jmsg.textContent = '';
          const table = findSensorsTable();
          // What discovery would store today, per command id — the same
          // composition the "add selection" flow uses, so diffs compare
          // stored values against exactly what a fresh add would write.
          const fresh = new Map();
          for (const room of body.rooms) {
            for (const eq of room.equipments) {
              for (const cmd of eq.cmds) {
                fresh.set(cmd.id, {
                  name: composeSensorName(cmd.name, eq.name, room.name),
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
            jd.diffs[id] = Object.entries(disc)
              .filter(([field, value]) => {
                // Stored kind may be '' — the skill treats that as numeric.
                const stored = field === 'kind' ? (row.kind || 'numeric') : String(row[field] ?? '');
                return stored !== String(value);
              })
              .map(([field, value]) => ({ field, value }));
          }
          // Sensors NOT in the table go through the existing tree flow.
          renderDiscoveryTree(tree, body.rooms, table);
          table?.rerender();
        },
      }),
```

- [ ] **Step 2: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js`
Expected: no output.
Desk-check the diff rules: a row `{id: 201, name: 'porte du garage', room: 'garage', kind: 'binary', on_label: 'ouverte', off_label: 'fermée', unit: ''}` against the discover fixture's Garage/Porte/État command produces NO diffs; renaming the room in the fixture produces exactly one `{field: 'room', …}` chip; a row whose id is in no fixture room lands in `jd.missing` and keeps its row (removal stays the user's explicit "Retirer" click).

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-admin/static/app.js
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: re-sync diffs sensor rows against fresh Jeedom discovery"
```

---

### Task 7: Workspace verification

**Files:** none new — this is the gate before calling the feature done.

- [ ] **Step 1: Full checks**

Run, in order:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
node --check crates/athena-voice-admin/static/app.js
```

Expected: all clean. Fix anything that surfaces (respecting `#![deny(warnings)]`), amend into the relevant commit or add a fixup commit with the Gekkotron identity.

- [ ] **Step 2: Post-ship note**

The live check on the GEEKOM (Lire on a real sensor, re-sync after renaming a room, duplicate chip on the six "température" rows) is the OWNER's manual step per the spec — do not attempt it from this environment (macOS Local Network restrictions). Surface it in the final report instead.

---

## Out of scope (per spec)

- Device/action commands and confirm-flagged execution (sub-project A).
- Editing phrases per sensor (custom aliases).
- Client-side phrase preview of UNSAVED edits — hints refresh on save only; the stale `·` marker is the honest stand-in.
