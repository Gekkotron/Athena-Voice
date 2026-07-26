# Jeedom Discovery, Rooms & Connection Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** "Tester la connexion" and "Découvrir les capteurs" buttons on the Jeedom admin form (host-side endpoints, opt-in checkbox tree), plus room-aware, enumerable, binary-state-capable voice queries.

**Architecture:** A Jeedom-specific module in `athena-voice-admin` calls the box's `jeeApi.php` server-side using the saved merged config (key never reaches the browser); the UI writes discovered sensors into the existing `sensors` list through the normal validated PUT. The skill's sensor entries gain optional `room`/`kind`/`on_label`/`off_label` fields and new pattern rules. Spec: `docs/superpowers/specs/2026-07-26-jeedom-discovery-design.md`.

**Tech Stack:** Rust edition 2024, axum 0.8, reqwest 0.12 (already an admin dep), wiremock 0.6 (workspace dev-dep), extism guest skills, vanilla JS.

## Global Constraints

- Every commit uses the Gekkotron identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`; plain imperative messages, no `feat:`/`fix:` prefixes.
- The Jeedom API key must never appear in any HTTP response body, any log line, or any tracked file. Every new admin endpoint sits under the existing `/api` token middleware.
- Clippy bar: ZERO NEW findings (`cargo clippy -p <crate> --all-targets --no-deps`; the workspace has known pre-existing debt in other crates). `cargo fmt` only on files you touch.
- After ANY `static/app.js` edit: `node --check crates/athena-voice-admin/static/app.js` must exit 0 (a straight apostrophe in a French string once broke the whole UI — use U+2019 `’` inside single-quoted strings).
- Backward compatibility: existing `sensors` configs (`{name,id,unit}` only) must keep parsing and matching exactly as today.
- Tests use `SqliteStore::open("sqlite::memory:")`; admin endpoint tests reuse the helpers in `crates/athena-voice-admin/tests/api.rs` (`test_deps()`, `get()`, `post()`).

---

### Task 1: Admin — `POST /api/skills/jeedom/test`

**Files:**
- Create: `crates/athena-voice-admin/src/jeedom.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (module + route + shared reqwest client in `AppState`)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: `AppState { store, skills, base_per_skill, … }`, `apply_settings`, `Store::skill_settings_for`.
- Produces: `POST /api/skills/jeedom/test` → 200 `{"status":"ok","version":"4.4.19"}` | `{"status":"unauthorized"}` | `{"status":"unreachable"}` | `{"status":"bad_response"}` | `{"status":"unconfigured"}`. Also `jeedom::resolved_config(&AppState) -> Option<JeedomCfg{base_url,api_key}>` reused by Task 2.

- [ ] **Step 1: Write the failing tests**

Append to `crates/athena-voice-admin/tests/api.rs` (wiremock is a workspace dev-dep — add `wiremock = { workspace = true }` to the admin crate's `[dev-dependencies]`):

```rust
async fn deps_with_jeedom_config(base_url: &str) -> (AdminDeps, String) {
    let (mut deps, token) = test_deps().await;
    deps.store
        .skill_setting_set("jeedom", "base_url", base_url, false)
        .await
        .unwrap();
    deps.store
        .skill_setting_set("jeedom", "api_key", "sekret-key-123", true)
        .await
        .unwrap();
    (deps, token)
}

#[tokio::test]
async fn jeedom_test_reports_ok_with_version() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/core/api/jeeApi.php"))
        .and(wiremock::matchers::query_param("type", "version"))
        .and(wiremock::matchers::query_param("apikey", "sekret-key-123"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("4.4.19"))
        .mount(&server)
        .await;
    let (deps, token) = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test", &token)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], "4.4.19");
    assert!(!String::from_utf8_lossy(&bytes).contains("sekret-key-123"),
        "api key must never be echoed");
}

#[tokio::test]
async fn jeedom_test_classifies_failures() {
    // unauthorized: Jeedom answers 200 with an error sentence, not a version.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("Clé API non valide"))
        .mount(&server)
        .await;
    let (deps, token) = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app.clone().oneshot(post("/api/skills/jeedom/test", &token)).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["status"], "unauthorized");

    // unreachable: nothing listens on the port (mock server dropped).
    let dead_uri = server.uri();
    drop(server);
    let (deps, token) = deps_with_jeedom_config(&dead_uri).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test", &token)).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["status"], "unreachable");
}

#[tokio::test]
async fn jeedom_test_unconfigured_and_auth_gated() {
    let (deps, token) = test_deps().await; // no jeedom config at all
    let app = router(deps);
    let unauth = app.clone().oneshot(
        Request::builder().method("POST").uri("/api/skills/jeedom/test")
            .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    let res = app.oneshot(post("/api/skills/jeedom/test", &token)).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["status"], "unconfigured");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin jeedom_test`
Expected: compile FAIL (route/module missing).

- [ ] **Step 3: Implement**

Create `crates/athena-voice-admin/src/jeedom.rs`:

```rust
//! Jeedom-specific admin endpoints: connection test and sensor discovery.
//! Host-side by design (spec 2026-07-26): the box is called with the SAVED
//! merged config, and the API key never travels to the browser.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use athena_voice_runtime::wasm::settings::apply_settings;

use crate::AppState;

pub(crate) struct JeedomCfg {
    pub base_url: String,
    pub api_key: String,
}

/// Saved merged config (base TOML + DB rows). `None` when base_url or
/// api_key is missing/empty — the endpoints answer `unconfigured`.
pub(crate) async fn resolved_config(state: &AppState) -> Option<JeedomCfg> {
    let rows = state.store.skill_settings_for("jeedom").await.ok()?;
    let base = state.base_per_skill.get("jeedom").cloned().unwrap_or_default();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let merged = apply_settings(&base, &pairs);
    let base_url = merged.config.get("base_url").filter(|s| !s.is_empty())?.clone();
    let api_key = merged.config.get("api_key").filter(|s| !s.is_empty())?.clone();
    Some(JeedomCfg {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
    })
}

fn status_json(status: &str) -> Response {
    Json(serde_json::json!({ "status": status })).into_response()
}

/// Version strings look like "4.4.19"; anything else that came back 2xx is
/// Jeedom's prose error for a bad key.
fn looks_like_version(body: &str) -> bool {
    let t = body.trim();
    !t.is_empty()
        && t.len() <= 32
        && t.chars().all(|c| c.is_ascii_digit() || c == '.')
        && t.contains('.')
}

pub(crate) async fn test_connection(State(state): State<AppState>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=version",
        cfg.base_url, cfg.api_key
    );
    let resp = match state.http.get(&url).timeout(Duration::from_secs(5)).send().await {
        Ok(r) => r,
        Err(_) => return status_json("unreachable"),
    };
    if !resp.status().is_success() {
        return status_json("bad_response");
    }
    let body = match resp.text().await {
        Ok(b) => b,
        Err(_) => return status_json("bad_response"),
    };
    if looks_like_version(&body) {
        Json(serde_json::json!({ "status": "ok", "version": body.trim() })).into_response()
    } else {
        status_json("unauthorized")
    }
}
```

In `crates/athena-voice-admin/src/lib.rs`:
- add `pub(crate) mod jeedom;`
- add a shared HTTP client to `AppState`: field `pub http: reqwest::Client`, initialized in `router()` as `reqwest::Client::new()` (one client, connection pooling; per-request `.timeout(...)` overrides).
- mount inside the `/api` router: `.route("/skills/jeedom/test", axum::routing::post(jeedom::test_connection))`. Route-collision note: the existing captures are `/skills/{name}/config|enable|disable` — the literal `jeedom/test` tail conflicts with none of them (axum 0.8 prefers literals anyway).

Error text rule: no upstream body content is ever included in our response (only the version string once it matched the strict digits-and-dots check) — Jeedom's prose could contain anything.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets --no-deps && cargo fmt -- crates/athena-voice-admin/src/jeedom.rs crates/athena-voice-admin/src/lib.rs`
Expected: all PASS, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin API: Jeedom connection test endpoint"
```

---

### Task 2: Admin — `POST /api/skills/jeedom/discover`

**Files:**
- Modify: `crates/athena-voice-admin/src/jeedom.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (route)
- Modify: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: `resolved_config`, `AppState.http` (Task 1).
- Produces: `POST /api/skills/jeedom/discover` → 200
  `{"status":"ok","rooms":[{"name":"Salon","equipments":[{"name":"Capteur","cmds":[{"id":142,"name":"Température","subtype":"numeric","unit":"°C","on_label":null,"off_label":null}]}]}]}`
  or the same failure statuses as Task 1. Only `type == "info"` commands appear; cmds with missing/empty names are skipped; response from Jeedom is capped at 4 MiB.

- [ ] **Step 1: Write the failing tests**

Append to `tests/api.rs`:

```rust
const FULLDATA_FIXTURE: &str = r#"[
  { "name": "Salon", "eqLogics": [
    { "name": "Capteur Xiaomi", "cmds": [
      { "id": "142", "name": "Température", "type": "info", "subType": "numeric", "unite": "°C" },
      { "id": 143, "name": "Rafraîchir", "type": "action", "subType": "other" },
      { "id": 144, "name": "", "type": "info", "subType": "numeric" }
    ] }
  ] },
  { "name": "Garage", "eqLogics": [
    { "name": "Porte", "cmds": [
      { "id": 201, "name": "État", "type": "info", "subType": "binary",
        "display": { "on_label": "ouverte", "off_label": "fermée" } }
    ] }
  ] }
]"#;

#[tokio::test]
async fn jeedom_discover_returns_info_command_tree() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::query_param("type", "fullData"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(FULLDATA_FIXTURE))
        .mount(&server)
        .await;
    let (deps, token) = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/discover", &token)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("sekret-key-123"));
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");
    let rooms = body["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 2);
    let salon_cmds = rooms[0]["equipments"][0]["cmds"].as_array().unwrap();
    assert_eq!(salon_cmds.len(), 1, "action + unnamed cmds filtered out");
    assert_eq!(salon_cmds[0]["id"], 142); // string "142" normalized to number
    assert_eq!(salon_cmds[0]["subtype"], "numeric");
    assert_eq!(salon_cmds[0]["unit"], "°C");
    let garage_cmd = &rooms[1]["equipments"][0]["cmds"][0];
    assert_eq!(garage_cmd["subtype"], "binary");
    assert_eq!(garage_cmd["on_label"], "ouverte");
    assert_eq!(garage_cmd["off_label"], "fermée");
}

#[tokio::test]
async fn jeedom_discover_bad_payload_is_bad_response() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("<html>login</html>"))
        .mount(&server)
        .await;
    let (deps, token) = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/discover", &token)).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["status"], "bad_response");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin jeedom_discover`
Expected: FAIL (404 — route missing).

- [ ] **Step 3: Implement**

Append to `jeedom.rs`:

```rust
use serde::Deserialize;
use serde::Serialize;

const FULLDATA_CAP_BYTES: usize = 4 * 1024 * 1024;

/// Jeedom's `id` arrives as a number or a numeric string depending on
/// version/plugin; normalize to u64.
fn cmd_id(v: &serde_json::Value) -> Option<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[derive(Serialize)]
pub(crate) struct DiscoveredCmd {
    id: u64,
    name: String,
    subtype: String,
    unit: Option<String>,
    on_label: Option<String>,
    off_label: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct DiscoveredEquipment {
    name: String,
    cmds: Vec<DiscoveredCmd>,
}

#[derive(Serialize)]
pub(crate) struct DiscoveredRoom {
    name: String,
    equipments: Vec<DiscoveredEquipment>,
}

/// Defensive walk of the fullData array: every field is optional, anything
/// malformed is skipped rather than fatal, and only `type == "info"`
/// commands with a non-empty name survive.
fn parse_fulldata(raw: &serde_json::Value) -> Option<Vec<DiscoveredRoom>> {
    let objects = raw.as_array()?;
    let mut rooms = Vec::new();
    for obj in objects {
        let room_name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut equipments = Vec::new();
        for eq in obj.get("eqLogics").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
            let eq_name = eq.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut cmds = Vec::new();
            for cmd in eq.get("cmds").and_then(|v| v.as_array()).unwrap_or(&Vec::new()) {
                if cmd.get("type").and_then(|v| v.as_str()) != Some("info") {
                    continue;
                }
                let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let Some(id) = cmd.get("id").and_then(cmd_id) else { continue };
                if name.is_empty() {
                    continue;
                }
                let display = cmd.get("display");
                let label = |key: &str| {
                    display
                        .and_then(|d| d.get(key))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                };
                cmds.push(DiscoveredCmd {
                    id,
                    name: name.to_string(),
                    subtype: cmd
                        .get("subType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("numeric")
                        .to_string(),
                    unit: cmd
                        .get("unite")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                    on_label: label("on_label"),
                    off_label: label("off_label"),
                });
            }
            if !cmds.is_empty() {
                equipments.push(DiscoveredEquipment { name: eq_name, cmds });
            }
        }
        if !equipments.is_empty() {
            rooms.push(DiscoveredRoom { name: room_name, equipments });
        }
    }
    Some(rooms)
}

pub(crate) async fn discover(State(state): State<AppState>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=fullData",
        cfg.base_url, cfg.api_key
    );
    let resp = match state.http.get(&url).timeout(Duration::from_secs(10)).send().await {
        Ok(r) => r,
        Err(_) => return status_json("unreachable"),
    };
    if !resp.status().is_success() {
        return status_json("bad_response");
    }
    let bytes = match resp.bytes().await {
        Ok(b) if b.len() <= FULLDATA_CAP_BYTES => b,
        _ => return status_json("bad_response"),
    };
    let Ok(raw) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return status_json("bad_response");
    };
    match parse_fulldata(&raw) {
        Some(rooms) => Json(serde_json::json!({ "status": "ok", "rooms": rooms })).into_response(),
        None => status_json("bad_response"),
    }
}
```

(Adjust the `unwrap_or(&Vec::new())` borrow pattern if clippy objects — `map_or_else` or a `static EMPTY: Vec<…>` alternative is fine; behavior is what's specified.) Remove the now-unused `Deserialize` import if nothing uses it.

Mount in `lib.rs`: `.route("/skills/jeedom/discover", axum::routing::post(jeedom::discover))`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin && cargo clippy -p athena-voice-admin --all-targets --no-deps`
Expected: PASS, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin API: Jeedom sensor discovery endpoint"
```

---

### Task 3: UI — test button, discovery tree, name composition

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js`
- Modify: `crates/athena-voice-admin/tests/api.rs` (only if an embedded-asset test needs re-blessing — content hash changes are fine, the MIME test doesn't pin content)

**Interfaces:**
- Consumes: the two endpoints above; the existing `renderDetail(skill)`, `listEditor` (its `<table>` exposes `getRows()`), `api()`, `el()`, `t()` helpers in app.js.
- Produces: on the jeedom detail screen only — a "Test connection" button with inline result and a "Discover sensors" flow that merges ticked commands into the sensors list editor.

- [ ] **Step 1: Extend the i18n dictionaries**

Add to BOTH `T.en` and `T.fr` in `app.js` (French values shown; en obvious equivalents — keep U+2019 for apostrophes):

```javascript
    // en
    test_connection: 'Test connection', testing: 'Testing…',
    jeedom_ok: 'Jeedom reachable, version ', jeedom_unauthorized: 'Invalid API key',
    jeedom_unreachable: 'Jeedom unreachable — check the URL', jeedom_bad_response: 'Unexpected reply — is this a Jeedom URL?',
    jeedom_unconfigured: 'Save the URL and API key first',
    discover: 'Discover sensors', discovering: 'Scanning…',
    add_selection: 'Add selection', nothing_discovered: 'No readable commands found',
    // fr
    test_connection: 'Tester la connexion', testing: 'Test en cours…',
    jeedom_ok: 'Jeedom joignable, version ', jeedom_unauthorized: 'Clé API invalide',
    jeedom_unreachable: 'Jeedom injoignable — vérifiez l’URL', jeedom_bad_response: 'Réponse inattendue — est-ce bien une URL Jeedom ?',
    jeedom_unconfigured: 'Enregistrez d’abord l’URL et la clé API',
    discover: 'Découvrir les capteurs', discovering: 'Analyse en cours…',
    add_selection: 'Ajouter la sélection', nothing_discovered: 'Aucune commande lisible trouvée',
```

- [ ] **Step 2: Name-composition helper (pure function, top level)**

```javascript
// French article lookup for composed sensor names; unknown rooms get no
// article ("température salon") — still fuzzy-matchable and editable.
const FR_ROOM_ARTICLES = {
  salon: 'du', bureau: 'du', garage: 'du', couloir: 'du', grenier: 'du', jardin: 'du',
  chambre: 'de la', cuisine: 'de la', terrasse: 'de la', cave: 'de la',
  'salle de bain': 'de la', 'salle à manger': 'de la', buanderie: 'de la',
};
function composeSensorName(cmdName, room) {
  const cmd = cmdName.toLowerCase();
  if (!room) return cmd;
  const r = room.toLowerCase();
  const article = /^[aeéèiouy]/.test(r) ? 'de l’' : FR_ROOM_ARTICLES[r];
  if (!article) return `${cmd} ${r}`;
  return article.endsWith('’') ? `${cmd} ${article}${r}` : `${cmd} ${article} ${r}`;
}
```

- [ ] **Step 3: listEditor row injection**

In `listEditor`, next to the existing `table.getRows = () => rows;`, add:

```javascript
  table.addRows = (newRows) => { rows.push(...newRows); render(); };
```

- [ ] **Step 4: Jeedom panel in renderDetail**

In `renderDetail(skill)`, after the widgets loop and before the Save button, insert (only for jeedom):

```javascript
  if (skill.name === 'jeedom') {
    const jmsg = el('p', { class: 'help' });
    const tree = el('div');
    const findSensorsTable = () =>
      widgets.find(([f]) => f.key === 'sensors')?.[1].querySelector('table');
    card.append(
      el('button', {
        class: 'quiet', text: t('test_connection'),
        onclick: async () => {
          jmsg.textContent = t('testing');
          const body = await (await api('/api/skills/jeedom/test', { method: 'POST' })).json();
          jmsg.textContent = body.status === 'ok'
            ? t('jeedom_ok') + body.version
            : t(`jeedom_${body.status}`);
        },
      }),
      el('button', {
        class: 'quiet', text: t('discover'),
        onclick: async () => {
          jmsg.textContent = t('discovering');
          const body = await (await api('/api/skills/jeedom/discover', { method: 'POST' })).json();
          if (body.status !== 'ok') { jmsg.textContent = t(`jeedom_${body.status}`); return; }
          jmsg.textContent = '';
          renderDiscoveryTree(tree, body.rooms, findSensorsTable());
        },
      }),
      jmsg, tree,
    );
  }
```

- [ ] **Step 5: The discovery tree renderer (top-level function)**

```javascript
function renderDiscoveryTree(container, rooms, sensorsTable) {
  const existing = new Set((sensorsTable?.getRows() || []).map((r) => Number(r.id)));
  const boxes = [];
  container.replaceChildren();
  if (!rooms.length) {
    container.append(el('p', { class: 'help', text: t('nothing_discovered') }));
    return;
  }
  for (const room of rooms) {
    const section = el('div', {}, el('label', { text: room.name || '—' }));
    for (const eq of room.equipments) {
      for (const cmd of eq.cmds) {
        const box = el('input', { type: 'checkbox' });
        box.checked = existing.has(cmd.id);
        box.disabled = existing.has(cmd.id); // already mapped — keep it in the table
        boxes.push({ box, cmd, room: room.name });
        const badge = cmd.subtype === 'binary' ? 'on/off' : (cmd.unit || '');
        section.append(el('div', { class: 'skill-row' },
          box,
          el('span', { class: 'name', text: `${eq.name} — ${cmd.name}` }),
          badge ? el('span', { class: 'badge', text: badge }) : '',
        ));
      }
    }
    container.append(section);
  }
  container.append(el('button', {
    text: t('add_selection'),
    onclick: () => {
      const picked = boxes.filter(({ box }) => box.checked && !box.disabled);
      sensorsTable?.addRows(picked.map(({ cmd, room }) => ({
        name: composeSensorName(cmd.name, room),
        id: cmd.id,
        unit: cmd.unit || '',
        room: (room || '').toLowerCase(),
        kind: cmd.subtype === 'binary' ? 'binary' : 'numeric',
        on_label: cmd.on_label || '',
        off_label: cmd.off_label || '',
      })));
      container.replaceChildren();
    },
  }));
}
```

Note: the sensors table's columns come from the skill's `config_schema` `item_fields` — Task 4 adds `room`/`kind`/`on_label`/`off_label` there, so the injected row objects line up with the columns. Task order within this plan guarantees Task 4 lands in the same session; the UI change is still safe standalone (extra keys not in `item_fields` are preserved in `rows` and serialized — they simply have no column until Task 4's schema ships).

- [ ] **Step 6: Verify**

Run: `node --check crates/athena-voice-admin/static/app.js` → exit 0.
Run: `cargo test -p athena-voice-admin` (assets re-embed) → all PASS.
Run: `cargo clippy -p athena-voice-admin --all-targets --no-deps` → clean.

- [ ] **Step 7: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Admin UI: Jeedom connection test and sensor discovery tree"
```

---

### Task 4: Skill — rooms, enumeration, binary states + schema + wasm rebuild

**Files:**
- Modify: `skills-jeedom/src/lib.rs`
- Modify (binary artifact, untracked): `skills/jeedom.wasm` via `./skills-jeedom/build.sh`

**Interfaces:**
- Consumes: existing `Sensor`, `sensors()`, `resolve_sensor`, `speak_reading`, `read_value`, `pattern_rules`, `config_schema` (full current source is in this file — read it first).
- Produces: backward-compatible `Sensor { name, id, unit, room, kind, on_label, off_label }` (all new fields `#[serde(default)]` `String`); new intents `jeedom.read.{id}` literal room phrasings and `jeedom.read_all.{metric}` enumeration; binary readings spoken via labels.

- [ ] **Step 1: Make the logic host-testable**

The current `sensors(ctx)` OnceCell can't be injected in tests. Refactor: every function that consumes the sensor list takes `&[Sensor]` (pure), and the thin wasm-facing layer passes `sensors(ctx)`:

- `fn rules_for(locale: &str, sensors: &[Sensor]) -> Vec<PatternRule>` — body of today's `pattern_rules` with `configured` replaced by the parameter.
- `fn resolve_in<'a>(list: &'a [Sensor], asked: &str) -> Option<&'a Sensor>` — body of `resolve_sensor`.
- Keep `speak_reading`/`read_value` as-is (they need HostCtx for HTTP; their *phrasing* moves to a pure `fn phrase_reading(sensor: &Sensor, value: &str, en: bool) -> String`).

If `cargo test` cannot run on the host in this crate (extism-pdk guest-only linkage), gate the `#[plugin_fn]` exports and `HostCtx`-touching code behind `#[cfg(target_arch = "wasm32")]` and keep the pure functions + tests target-independent. Record which way it went in your report.

- [ ] **Step 2: Write the failing tests (in `skills-jeedom/src/lib.rs`, `#[cfg(test)]`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, id: u64, unit: &str, room: &str, kind: &str, on: &str, off: &str) -> Sensor {
        Sensor {
            name: name.into(), id, unit: unit.into(), room: room.into(),
            kind: kind.into(), on_label: on.into(), off_label: off.into(),
        }
    }

    #[test]
    fn old_config_shape_still_parses() {
        let raw = r#"[{"name":"température du salon","id":142,"unit":"degrés"}]"#;
        let v: Vec<Sensor> = serde_json::from_str(raw).unwrap();
        assert_eq!(v[0].room, "");
        assert_eq!(v[0].kind, "");
        assert!(v[0].on_label.is_empty() && v[0].off_label.is_empty());
    }

    #[test]
    fn room_query_phrases_are_generated() {
        let list = vec![s("température du salon", 142, "degrés", "salon", "numeric", "", "")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules.iter().flat_map(|r| r.phrases.iter().map(String::as_str)).collect();
        assert!(all.contains(&"quelle température dans le salon"), "got: {all:?}");
        assert!(all.contains(&"température dans le salon"));
        // the room rule routes to the literal per-sensor intent
        assert!(rules.iter().any(|r| r.intent == "jeedom.read.142"
            && r.phrases.iter().any(|p| p.contains("dans le salon"))));
    }

    #[test]
    fn enumeration_rule_per_metric() {
        let list = vec![
            s("température du salon", 142, "degrés", "salon", "numeric", "", ""),
            s("température de la chambre", 150, "degrés", "chambre", "numeric", "", ""),
            s("humidité du salon", 143, "%", "salon", "numeric", "", ""),
        ];
        let rules = rules_for("fr", &list);
        let enum_rules: Vec<_> = rules.iter().filter(|r| r.intent.starts_with("jeedom.read_all.")).collect();
        assert_eq!(enum_rules.len(), 2, "one per distinct metric: température, humidité");
        assert!(enum_rules.iter().any(|r| r.intent == "jeedom.read_all.température"
            && r.phrases.contains(&"toutes les températures".to_string())));
    }

    #[test]
    fn metric_word_strips_room_suffix() {
        assert_eq!(metric_of(&s("température du salon", 1, "", "salon", "", "", "")), "température");
        assert_eq!(metric_of(&s("température de la chambre", 1, "", "chambre", "", "", "")), "température");
        assert_eq!(metric_of(&s("capteur exotique", 1, "", "", "", "", "")), "capteur exotique");
    }

    #[test]
    fn binary_reading_uses_labels_and_falls_back() {
        let door = s("porte du garage", 201, "", "garage", "binary", "ouverte", "fermée");
        assert_eq!(phrase_reading(&door, "1", false), "la porte du garage est ouverte");
        assert_eq!(phrase_reading(&door, "0", false), "la porte du garage est fermée");
        let plain = s("présence salon", 202, "", "salon", "binary", "", "");
        assert_eq!(phrase_reading(&plain, "1", false), "la présence salon est activé");
        let temp = s("température du salon", 142, "degrés", "salon", "numeric", "", "");
        assert_eq!(phrase_reading(&temp, "21.5", false), "la température du salon est de 21.5 degrés");
        assert_eq!(phrase_reading(&temp, "21.5", true), "the température du salon is 21.5 degrés");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd skills-jeedom && cargo test`
Expected: compile FAIL (new fields/functions missing). If host compilation itself fails on extism-pdk, apply the `#[cfg(target_arch = "wasm32")]` gating from Step 1 first, then re-run.

- [ ] **Step 4: Implement**

1. `Sensor` gains the new fields, all `#[serde(default)] String`: `room`, `kind`, `on_label`, `off_label`. (`kind` empty == numeric; only the exact string `"binary"` switches behavior — matches the schema Task 4 ships.)

2. `fn metric_of(sensor: &Sensor) -> String` — the sensor's "metric word": name with a trailing `(du|de la|de l’|de|dans le|dans la)? <room>` stripped when `room` is non-empty and the name ends with the room word; else the full name:

```rust
fn metric_of(sensor: &Sensor) -> String {
    let name = sensor.name.trim().to_lowercase();
    let room = sensor.room.trim().to_lowercase();
    if room.is_empty() || !name.ends_with(&room) {
        return name;
    }
    let head = name[..name.len() - room.len()].trim_end();
    let head = ["du", "de la", "de l’", "de l'", "de", "dans le", "dans la"]
        .iter()
        .find_map(|art| head.strip_suffix(art))
        .unwrap_or(head)
        .trim_end();
    if head.is_empty() { name } else { head.to_string() }
}
```

3. `rules_for(locale, sensors)`: keep every existing rule, then
   - for each sensor with non-empty `room` (fr): push onto that sensor's existing literal rule (`jeedom.read.{id}`) the phrases `format!("quelle {metric} dans le {room}")`, `format!("quelle {metric} dans la {room}")`, `format!("{metric} dans le {room}")`, `format!("{metric} dans la {room}")` (both genders — the matcher's fuzzy tolerance is helped by having both; en: `format!("{metric} in the {room}")`, `format!("what is the {metric} in the {room}")`).
   - group sensors by `metric_of`; for each metric with ≥1 sensor, add `PatternRule { intent: format!("jeedom.read_all.{metric}"), phrases: fr → ["toutes les {metric}s" (naive plural: append 's' unless already ending in 's'), "toutes les {metric}"], en → ["all {metric}s", "all the {metric}s"], slots: vec![] }`.

4. `phrase_reading(sensor, value, en) -> String`:

```rust
fn phrase_reading(sensor: &Sensor, value: &str, en: bool) -> String {
    if sensor.kind == "binary" {
        let on = value == "1" || value.eq_ignore_ascii_case("true");
        let label = if on {
            if sensor.on_label.is_empty() { if en { "on" } else { "activé" } } else { &sensor.on_label }
        } else if sensor.off_label.is_empty() {
            if en { "off" } else { "désactivé" }
        } else {
            &sensor.off_label
        };
        return if en {
            format!("the {} is {label}", sensor.name)
        } else {
            format!("la {} est {label}", sensor.name)
        };
    }
    let unit = if sensor.unit.is_empty() { String::new() } else { format!(" {}", sensor.unit) };
    if en {
        format!("the {} is {value}{unit}", sensor.name)
    } else {
        format!("la {} est de {value}{unit}", sensor.name)
    }
}
```

   `speak_reading` becomes: `read_value(ctx, sensor)` then `Ok(SkillResponse::speak(phrase_reading(sensor, &value, en)))` (error branch unchanged).

5. `handle`: add the enumeration branch before the slot fallback:

```rust
        if let Some(metric) = intent.name.strip_prefix("jeedom.read_all.") {
            let list = sensors(ctx);
            let matching: Vec<&Sensor> =
                list.iter().filter(|s| metric_of(s) == metric || s.name.contains(metric)).collect();
            if matching.is_empty() {
                return Ok(SkillResponse::speak(if en {
                    format!("no sensor matches {metric}")
                } else {
                    format!("aucun capteur ne correspond à {metric}")
                }));
            }
            let mut clauses = Vec::new();
            for sensor in matching {
                let place = if sensor.room.is_empty() { sensor.name.clone() } else { sensor.room.clone() };
                match read_value(ctx, sensor) {
                    Ok(v) => clauses.push(format!("{place} {v}{}",
                        if sensor.unit.is_empty() { String::new() } else { format!(" {}", sensor.unit) })),
                    Err(()) => clauses.push(if en { format!("{place} unavailable") } else { format!("{place} indisponible") }),
                }
            }
            return Ok(SkillResponse::speak(clauses.join(", ")));
        }
```

6. `config_schema`: extend the sensors `item_fields` with `room` (String), `kind` (String), `on_label` (String), `off_label` (String); update the `help` text to "Spoken name → Jeedom command id; room/kind filled by discovery".

7. Update the module doc comment's config example to show the new optional fields.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd skills-jeedom && cargo test && cargo clippy --all-targets 2>&1 | tail -5`
Expected: all PASS; no NEW clippy findings (one pre-existing `collapsible_if` is known).

- [ ] **Step 6: Rebuild the wasm**

Run: `./skills-jeedom/build.sh`
Expected: "Copied to ../skills/jeedom.wasm". Verify the running skill reloads (if `serve` is up, re-upload via the UI or restart) — note in the report either way.

- [ ] **Step 7: Commit**

```bash
git add skills-jeedom/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "Jeedom skill: room queries, enumeration, and spoken binary states"
```

---

### Task 5: README + final verification

**Files:**
- Modify: `README.md` (Jeedom section)

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Update the README's Jeedom section**

Replace the manual command-id instructions with the discovery flow (keep the API-key creation steps):

```markdown
### Enabling the Jeedom skill

1. In Jeedom: Settings → System → Configuration → API — copy (or create)
   an API key, ideally one restricted to the commands you want to expose.
2. In the Athena-Voice web UI (`http://127.0.0.1:8080`), open **jeedom**,
   fill in the Jeedom URL and API key, and **save**.
3. Click **Tester la connexion** — you should see the Jeedom version.
4. Click **Découvrir les capteurs**, tick the sensors you want by room,
   then **Ajouter la sélection** and save. Spoken names are pre-composed
   ("température du salon") and editable.
5. Ask by voice: "quelle est la température du salon", "quelle température
   dans la chambre", "toutes les températures", or for door/presence
   sensors "quelle est la porte du garage" → "la porte du garage est
   ouverte".
```

- [ ] **Step 2: Full verification battery**

Run: `cargo test --workspace` (known flaky watcher test: retry in isolation if it trips)
Run: `cargo build --workspace`
Run: `node --check crates/athena-voice-admin/static/app.js`
Run: `git grep -E "[A-Za-z0-9]{40,}" -- '*.toml' || echo CLEAN` → CLEAN
Run: `cd skills-jeedom && cargo test`
Expected: all green.

- [ ] **Step 3: Commit**

```bash
git add README.md
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com \
  commit -m "README: Jeedom setup via discovery in the web UI"
```
