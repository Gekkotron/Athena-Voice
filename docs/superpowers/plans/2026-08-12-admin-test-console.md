# Admin Test Console Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A test-only input field in the admin web UI that sends a typed command through the real MQTT satellite path and shows the assistant's answer, with client-side history of the last commands.

**Architecture:** New `POST /api/test-command` endpoint in `athena-voice-admin` opens a throwaway MQTT session as satellite `admin-ui` (start → text → collect `tts/text` → end), exactly the contract `athena-voice-client` uses. The UI gets a "Test console" card; history lives in `localStorage` only. Spec: `docs/superpowers/specs/2026-08-12-admin-test-console-design.md`.

**Tech Stack:** Rust (axum 0.8-style routing, rumqttc 0.24, tokio), vanilla JS admin UI (`static/app.js`).

## Global Constraints

- Every admin crate file starts under `#![deny(warnings)]` — code must be warning-free.
- Git identity: commit with `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …` (never edit `.git/config`), and end each commit message with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- If cargo complains the pinned toolchain is missing, prefix commands with `RUSTUP_TOOLCHAIN=1.95`.
- Working directory: repo root `/Users/julienhuguel/superconductor/projects/Athena-Voice`.
- MQTT topic layout is the existing contract in `crates/athena-voice-runtime/src/mqtt/topics.rs` (`athena/sat/<sat>/session/<sid>/{start,text,end,tts/text,done}`) — do not invent new topics.

---

### Task 1: MQTT text-session function in the admin crate

**Files:**
- Modify: `crates/athena-voice-admin/Cargo.toml`
- Create: `crates/athena-voice-admin/src/test_command.rs`
- Modify: `crates/athena-voice-admin/src/lib.rs` (module + re-export only)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `pub struct AdminMqttConfig { pub host: String, pub port: u16, pub username: Option<String>, pub password: Option<String> }` (Clone, Debug); `pub(crate) enum TestCommandError { Connect(String), Timeout }` (Debug); `pub(crate) async fn run_text_session(cfg: &AdminMqttConfig, text: &str, locale: &str) -> Result<String, TestCommandError>`. Task 2 calls `run_text_session` and re-exports `AdminMqttConfig` from `lib.rs`.

- [ ] **Step 1: Move rumqttc to real dependencies**

In `crates/athena-voice-admin/Cargo.toml`, add to `[dependencies]` (rumqttc is currently only in `[dev-dependencies]` — remove it from there since dev-deps inherit real deps):

```toml
rumqttc = { workspace = true }
```

- [ ] **Step 2: Write the failing test with a stub**

Create `crates/athena-voice-admin/src/test_command.rs`:

```rust
//! Test console: drives a one-shot text session over MQTT, the same
//! satellite topic contract an Android app or `athena-voice-client` uses
//! (see `crates/athena-voice-runtime/src/mqtt/topics.rs`).

use std::time::Duration;

/// Broker coordinates for the test console, mirroring the CLI's `[mqtt]`
/// config. `None` in `AdminDeps.mqtt` disables the endpoint (503).
#[derive(Clone, Debug)]
pub struct AdminMqttConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug)]
pub(crate) enum TestCommandError {
    /// Broker unreachable / protocol error → 502.
    Connect(String),
    /// No answer before the deadline → 504.
    Timeout,
}

/// LLM streaming can flush several `tts/text` segments; a quiet gap after
/// the first one is treated as end-of-answer (same heuristic as
/// `athena-voice-client`).
const ANSWER_QUIET: Duration = Duration::from_millis(1200);
const SESSION_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn run_text_session(
    cfg: &AdminMqttConfig,
    text: &str,
    locale: &str,
) -> Result<String, TestCommandError> {
    // Reference everything the real body will use — the crate is
    // #![deny(warnings)], so an unused const/variant would turn the red
    // step into a compile error instead of a failing test.
    let _ = (text, locale, ANSWER_QUIET, SESSION_TIMEOUT);
    let _ = TestCommandError::Connect(cfg.host.clone());
    let _ = TestCommandError::Timeout;
    todo!("implemented in the next step")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_broker_reports_connect_error() {
        let cfg = AdminMqttConfig {
            host: "127.0.0.1".into(),
            port: 1, // nothing listens here — refused immediately
            username: None,
            password: None,
        };
        let err = run_text_session(&cfg, "hello", "en").await.unwrap_err();
        assert!(matches!(err, TestCommandError::Connect(_)));
    }
}
```

Register the module in `crates/athena-voice-admin/src/lib.rs` next to the other modules (line 4-6):

```rust
pub(crate) mod test_command;

pub use test_command::AdminMqttConfig;
```

(the `pub use` goes after the `use` block, near `AdminDeps`).

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p athena-voice-admin unreachable_broker -- --nocapture`
Expected: FAIL with a panic from `todo!` ("implemented in the next step").

- [ ] **Step 4: Implement the session loop**

Replace the `run_text_session` stub body (keep the signature) with:

```rust
pub(crate) async fn run_text_session(
    cfg: &AdminMqttConfig,
    text: &str,
    locale: &str,
) -> Result<String, TestCommandError> {
    use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};

    let sid = uuid::Uuid::new_v4();
    let base = format!("athena/sat/admin-ui/session/{sid}");

    // Unique client id: concurrent test requests and the runtime's own
    // MQTT client must never collide on the broker.
    let mut opts = MqttOptions::new(
        format!("athena-admin-test-{}-{}", std::process::id(), &sid.to_string()[..8]),
        &cfg.host,
        cfg.port,
    );
    opts.set_keep_alive(Duration::from_secs(15));
    // TTS audio chunks (~9 KiB PCM) arrive on our wildcard subscription —
    // rumqttc's 10 KiB default cap is too close.
    opts.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
    if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
        opts.set_credentials(u, p);
    }
    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    client
        .subscribe(format!("{base}/#"), QoS::AtLeastOnce)
        .await
        .map_err(|e| TestCommandError::Connect(e.to_string()))?;

    let deadline = tokio::time::Instant::now() + SESSION_TIMEOUT;
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut started = false;
    let mut segments: Vec<String> = Vec::new();
    let mut last_segment_at = tokio::time::Instant::now();

    let result = loop {
        let ev = tokio::select! {
            ev = eventloop.poll() => ev,
            _ = tick.tick() => {
                if !segments.is_empty() && last_segment_at.elapsed() > ANSWER_QUIET {
                    break Ok(segments.join(" "));
                }
                continue;
            }
            () = tokio::time::sleep_until(deadline) => {
                break if segments.is_empty() {
                    Err(TestCommandError::Timeout)
                } else {
                    Ok(segments.join(" "))
                };
            }
        };
        match ev {
            Ok(MqttEvent::Incoming(Packet::SubAck(_))) if !started => {
                // Subscription is live — safe to open the session now.
                started = true;
                let start = client
                    .publish(
                        format!("{base}/start"),
                        QoS::AtLeastOnce,
                        false,
                        serde_json::json!({ "locale": locale }).to_string(),
                    )
                    .await;
                let text_pub = client
                    .publish(format!("{base}/text"), QoS::AtLeastOnce, false, text.to_string())
                    .await;
                if let Err(e) = start.and(text_pub) {
                    break Err(TestCommandError::Connect(e.to_string()));
                }
            }
            Ok(MqttEvent::Incoming(Packet::Publish(p))) => {
                if p.topic == format!("{base}/tts/text") {
                    segments.push(String::from_utf8_lossy(&p.payload).to_string());
                    last_segment_at = tokio::time::Instant::now();
                } else if p.topic == format!("{base}/done") {
                    break Ok(segments.join(" "));
                }
            }
            Ok(_) => {}
            Err(e) => break Err(TestCommandError::Connect(e.to_string())),
        }
    };

    // Close the session server-side even on failure — otherwise it lingers
    // until the session manager's idle reaper. Publishing only ENQUEUES;
    // poll until the broker acks (or give up after 1 s), then disconnect.
    let _ = client
        .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
        .await;
    let flush_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        tokio::select! {
            ev = eventloop.poll() => match ev {
                Ok(MqttEvent::Incoming(Packet::PubAck(_))) | Err(_) => break,
                Ok(_) => {}
            },
            () = tokio::time::sleep_until(flush_deadline) => break,
        }
    }
    let _ = client.disconnect().await;
    result
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p athena-voice-admin unreachable_broker`
Expected: PASS (connection refused surfaces as `Connect` well before the 10 s deadline).

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-admin/Cargo.toml crates/athena-voice-admin/src/test_command.rs crates/athena-voice-admin/src/lib.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin: one-shot MQTT text session for the test console

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: POST /api/test-command endpoint

**Files:**
- Modify: `crates/athena-voice-admin/Cargo.toml`
- Modify: `crates/athena-voice-admin/src/lib.rs`
- Modify: `crates/athena-voice-admin/src/test_command.rs`
- Test: `crates/athena-voice-admin/tests/api.rs`

**Interfaces:**
- Consumes: `run_text_session`, `AdminMqttConfig`, `TestCommandError` from Task 1.
- Produces: `AdminDeps.mqtt: Option<AdminMqttConfig>` (new public field — Task 3 sets it from the CLI); route `POST /api/test-command` accepting `{"text": "...", "locale": "fr"}` and returning `200 {"answer": "..."}` / `400|502|503|504 {"error": "..."}` (the UI in Task 4 reads `body.answer` and `body.error`).

- [ ] **Step 1: Add the athena-voice-core dependency**

`Locale::new` (the validator the runtime itself uses) lives in `athena_voice_core::ids`. In `crates/athena-voice-admin/Cargo.toml` `[dependencies]`, add:

```toml
athena-voice-core = { workspace = true }
```

- [ ] **Step 2: Write the failing router tests**

Append to `crates/athena-voice-admin/tests/api.rs` (it already has `test_deps()`, `get()`, and the `header` import):

```rust
fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_command_rejects_empty_text() {
    let app = router(test_deps().await);
    let res = app
        .oneshot(post_json(
            "/api/test-command",
            serde_json::json!({ "text": "   " }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert!(body_json(res).await["error"].is_string());
}

#[tokio::test]
async fn test_command_rejects_invalid_locale() {
    let app = router(test_deps().await);
    let res = app
        .oneshot(post_json(
            "/api/test-command",
            serde_json::json!({ "text": "hello", "locale": "français" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_command_without_mqtt_returns_503() {
    // test_deps() leaves `mqtt: None` — the endpoint must refuse cleanly.
    let app = router(test_deps().await);
    let res = app
        .oneshot(post_json(
            "/api/test-command",
            serde_json::json!({ "text": "hello" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(body_json(res).await["error"].is_string());
}
```

Also add `mqtt: None,` to the `AdminDeps` literal in `test_deps()` — the struct gains the field in the next step and the test file must initialize it.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p athena-voice-admin --test api test_command`
Expected: FAIL to compile (`AdminDeps` has no field `mqtt`, no `/api/test-command` route). A compile failure is this cycle's "red".

- [ ] **Step 4: Implement the endpoint**

In `crates/athena-voice-admin/src/lib.rs`:

1. Add the field to `AdminDeps` (after `bundled_dir`):

```rust
    /// Broker for the test console; `None` disables `POST /api/test-command`.
    pub mqtt: Option<AdminMqttConfig>,
```

2. Add the same field to `AppState`:

```rust
    pub mqtt: Option<AdminMqttConfig>,
```

and populate it in `router()`: `mqtt: deps.mqtt,`.

3. Register the route next to the others in `router()`:

```rust
        .route(
            "/test-command",
            axum::routing::post(test_command::test_command),
        )
```

In `crates/athena-voice-admin/src/test_command.rs`, add the handler:

```rust
use athena_voice_core::ids::Locale;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::AppState;

#[derive(serde::Deserialize)]
pub(crate) struct TestCommandReq {
    text: String,
    #[serde(default)]
    locale: Option<String>,
}

/// POST /api/test-command — test console: run one text command through the
/// satellite path and return the spoken answer as text.
pub(crate) async fn test_command(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<TestCommandReq>,
) -> Response {
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "text must not be empty");
    }
    let locale = req.locale.unwrap_or_else(|| "en".to_string());
    if Locale::new(locale.as_str()).is_err() {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid locale (expected e.g. \"fr\" or \"fr-FR\")",
        );
    }
    let Some(cfg) = state.mqtt.as_ref() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "MQTT is not configured on this admin server",
        );
    };
    match run_text_session(cfg, &text, &locale).await {
        Ok(answer) => axum::Json(serde_json::json!({ "answer": answer })).into_response(),
        Err(TestCommandError::Connect(e)) => {
            err(StatusCode::BAD_GATEWAY, &format!("MQTT broker error: {e}"))
        }
        Err(TestCommandError::Timeout) => err(
            StatusCode::GATEWAY_TIMEOUT,
            "no answer within 10s — is `serve` running with skills loaded?",
        ),
    }
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p athena-voice-admin`
Expected: all admin tests PASS (the three new ones plus the existing suite — the existing tests exercise `test_deps()` which now carries `mqtt: None`).

- [ ] **Step 6: Commit**

```bash
git add crates/athena-voice-admin
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin API: POST /api/test-command runs a text command over MQTT

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Wire the broker config through `serve`

**Files:**
- Modify: `crates/athena-voice-cli/src/serve.rs` (in `spawn_admin_ui`, ~line 91)

**Interfaces:**
- Consumes: `AdminDeps.mqtt` and `athena_voice_admin::AdminMqttConfig` from Task 2.
- Produces: nothing new — the running `serve` now exposes a working test console.

- [ ] **Step 1: Fill the new field**

In `spawn_admin_ui` in `crates/athena-voice-cli/src/serve.rs`, extend the `AdminDeps` literal (`cfg.mqtt` is the CLI's `[mqtt]` config — host/port/username/password fields already exist):

```rust
    let admin_deps = athena_voice_admin::AdminDeps {
        store,
        skills: runtime.skills.clone(),
        base_per_skill,
        bundled_dir: cfg.skills.bundled_dir.clone(),
        mqtt: Some(athena_voice_admin::AdminMqttConfig {
            host: cfg.mqtt.host.clone(),
            port: cfg.mqtt.port,
            username: cfg.mqtt.username.clone(),
            password: cfg.mqtt.password.clone(),
        }),
    };
```

- [ ] **Step 2: Verify the workspace still builds and tests pass**

Run: `cargo build -p athena-voice-cli && cargo test -p athena-voice-admin -p athena-voice-cli`
Expected: build OK, tests PASS. (If any other `AdminDeps` construction site fails to compile, add `mqtt: None` there.)

- [ ] **Step 3: Commit**

```bash
git add crates/athena-voice-cli/src/serve.rs
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "serve: pass [mqtt] broker config to the admin test console

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Test console card in the admin UI

**Files:**
- Modify: `crates/athena-voice-admin/static/app.js`
- Modify: `crates/athena-voice-admin/static/style.css`

**Interfaces:**
- Consumes: `POST /api/test-command` from Task 2 (`{"text", "locale"}` → `{"answer"}` or `{"error"}`), plus app.js's existing `el()`, `api()`, `t()`/`T`, `lang` helpers.
- Produces: a `testConsoleCard()` function appended to the page by `renderList()`.

- [ ] **Step 1: Add the i18n strings**

In `crates/athena-voice-admin/static/app.js`, extend BOTH locale objects in `T` (keep the existing keys untouched). In `en`:

```js
    test_console: 'Test console', test_console_help: 'Send a text command to the assistant — test tool, nothing is stored server-side.',
    send: 'Send', sending: 'Sending…', network_error: 'network error',
```

In `fr`:

```js
    test_console: 'Console de test', test_console_help: 'Envoyer une commande texte à l’assistant — outil de test, rien n’est stocké côté serveur.',
    send: 'Envoyer', sending: 'Envoi…', network_error: 'erreur réseau',
```

- [ ] **Step 2: Add the card**

Add above `renderList()` in `app.js`:

```js
// --- Test console (test tool): one-shot text command over the satellite
// path, with a client-side history of the last commands. ---

const TEST_HISTORY_KEY = 'athena-test-history';
const TEST_HISTORY_MAX = 20;

function loadTestHistory() {
  try {
    const h = JSON.parse(localStorage.getItem(TEST_HISTORY_KEY));
    return Array.isArray(h) ? h.filter((x) => typeof x === 'string') : [];
  } catch { return []; }
}

// Most recent first, deduped, capped.
function pushTestHistory(cmd) {
  const h = [cmd, ...loadTestHistory().filter((c) => c !== cmd)].slice(0, TEST_HISTORY_MAX);
  localStorage.setItem(TEST_HISTORY_KEY, JSON.stringify(h));
}

function testConsoleCard() {
  const input = el('input', { type: 'text', autocomplete: 'off', class: 'test-input' });
  const btn = el('button', { text: t('send') });
  const out = el('p', { class: 'test-answer' });
  const historyList = el('div', { class: 'test-history' });
  let histIdx = -1; // -1 = editing a fresh draft
  let draft = '';

  const renderHistory = () => {
    historyList.replaceChildren(...loadTestHistory().map((cmd) =>
      el('span', {
        class: 'badge test-history-item', text: cmd,
        onclick: () => { input.value = cmd; histIdx = -1; input.focus(); },
      })));
  };

  const submit = async () => {
    const text = input.value.trim();
    if (!text || input.disabled) return;
    input.disabled = true; btn.disabled = true;
    out.textContent = t('sending');
    try {
      const res = await api('/api/test-command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text, locale: lang }),
      });
      const body = await res.json().catch(() => ({}));
      out.textContent = res.ok ? (body.answer || '—') : (body.error || `HTTP ${res.status}`);
    } catch { out.textContent = t('network_error'); }
    input.disabled = false; btn.disabled = false;
    pushTestHistory(text); histIdx = -1; draft = '';
    renderHistory();
    input.value = ''; input.focus();
  };

  // Shell-style recall: ArrowUp walks back through history, ArrowDown
  // forward and finally back to the unsent draft.
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); submit(); return; }
    const h = loadTestHistory();
    if (e.key === 'ArrowUp') {
      if (!h.length) return;
      e.preventDefault();
      if (histIdx === -1) draft = input.value;
      histIdx = Math.min(histIdx + 1, h.length - 1);
      input.value = h[histIdx];
    } else if (e.key === 'ArrowDown') {
      if (histIdx === -1) return;
      e.preventDefault();
      histIdx -= 1;
      input.value = histIdx === -1 ? draft : h[histIdx];
    }
  });
  btn.addEventListener('click', submit);
  renderHistory();

  return el('section', { class: 'card' },
    el('h2', { text: t('test_console') }),
    el('p', { class: 'test-help', text: t('test_console_help') }),
    el('div', { class: 'test-row' }, input, btn),
    out,
    historyList,
  );
}
```

Then make `renderList()` append it — change its last line (currently `app.replaceChildren(list, await uploadCard());`) to:

```js
  app.replaceChildren(list, await uploadCard(), testConsoleCard());
```

- [ ] **Step 3: Style the card**

Append to `crates/athena-voice-admin/static/style.css` (matches the file's plain-CSS style):

```css
/* Test console */
.test-row { display: flex; gap: .5rem; }
.test-row .test-input { flex: 1; }
.test-help, .test-answer { opacity: .8; }
.test-history { margin-top: .5rem; display: flex; flex-wrap: wrap; gap: .35rem; }
.test-history-item { cursor: pointer; }
```

- [ ] **Step 4: Verify the embedded assets still build and tests pass**

The static files are embedded via `include_dir!`, so a rebuild picks them up:

Run: `cargo test -p athena-voice-admin && cargo build -p athena-voice-cli`
Expected: PASS / build OK.

- [ ] **Step 5: Commit**

```bash
git add crates/athena-voice-admin/static
git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit -m "Admin UI: test console card with localStorage command history

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Manual verification (human, after all tasks)

Not executable by a headless worker — needs a live broker + runtime:

1. Start the broker and `athena-voice serve` with skills loaded (as for `athena-voice-client`).
2. Open the admin UI, scroll to "Test console" / "Console de test".
3. Type a known command (e.g. a Jeedom sensor phrase), press Enter → the answer text appears; on a stopped runtime the field reports the 504 message instead.
4. Reload the page → the history chips persist; ArrowUp recalls the last command.
