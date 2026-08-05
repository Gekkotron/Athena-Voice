use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use athena_voice_admin::{AdminDeps, router};
use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::{SkillConfig, SkillDeps, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tokio::sync::broadcast;
use tower::ServiceExt; // oneshot

async fn test_store() -> Arc<dyn Store> {
    Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap())
}

async fn test_deps() -> AdminDeps {
    AdminDeps {
        store: test_store().await,
        skills: None,
        base_per_skill: HashMap::new(),
        bundled_dir: None,
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn skills_list_masks_secrets_and_shows_disabled() {
    let mut deps = test_deps().await;
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
    deps.store
        .skill_setting_set("jeedom", "api_key", "s3cret", true)
        .await
        .unwrap();
    deps.store
        .skill_setting_set("jeedom", "base_url", "http://db", false)
        .await
        .unwrap();
    deps.store.skill_enabled_set("jeedom", false).await.unwrap();

    let app = router(deps);
    let res = app.oneshot(get("/api/skills")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !body.to_string().contains("s3cret"),
        "secret value must never be echoed"
    );

    let jeedom = body
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "jeedom")
        .expect("jeedom listed even though unloaded");
    assert_eq!(jeedom["enabled"], false);
    assert_eq!(jeedom["loaded"], false);
    assert_eq!(jeedom["config"]["api_key"]["kind"], "secret");
    assert_eq!(jeedom["config"]["api_key"]["set"], true);
    assert_eq!(jeedom["config"]["base_url"]["value"], "http://db"); // DB wins
}

#[tokio::test]
async fn put_config_persists_and_rejects_invalid() {
    let deps = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);

    // No schema (skill not loaded) → free-form values accepted.
    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"values":{"base_url":"http://192.168.1.91"}}"#,
        ))
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
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"nope": 1}"#))
        .unwrap();
    let res = app.oneshot(bad).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY); // axum Json rejection
}

#[tokio::test]
async fn put_config_rejects_reserved_key_before_persisting() {
    // Schema validation can't fire in this test setup (skills: None, so no
    // loaded schema) — that path is covered at the unit level in
    // validate.rs. This pins the reject-before-persist ordering using the
    // `$`-reserved-key path instead, which runs regardless of schema.
    let deps = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"values":{"$http_allowlist":"[]"}}"#))
        .unwrap();
    let res = app.oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let rows = store.skill_settings_for("jeedom").await.unwrap();
    assert!(
        rows.is_empty(),
        "reserved-key write must be rejected before anything is persisted"
    );
}

fn post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn enable_disable_toggles_state() {
    let deps = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);
    let res = app
        .clone()
        .oneshot(post("/api/skills/timer/disable"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        store.skills_disabled().await.unwrap(),
        vec!["timer".to_string()]
    );
    let res = app.oneshot(post("/api/skills/timer/enable")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(store.skills_disabled().await.unwrap().is_empty());
}

#[tokio::test]
async fn upload_rejects_bad_names() {
    let deps = test_deps().await;
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
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn put_config_rejects_traversal_name() {
    // Percent-encoded `/` inside the path segment: axum's router matches on
    // the raw (still-encoded) path, so this stays a single `{name}` segment,
    // but the `Path` extractor hands the handler the decoded `/etc/evil` —
    // which must be rejected by `valid_skill_name` before it ever reaches a
    // store write or `dir.join(...)`.
    let deps = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);
    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/%2Fetc%2Fevil/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"values":{"x":"y"}}"#))
        .unwrap();
    let res = app.oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let rows = store.skill_settings_all().await.unwrap();
    assert!(
        rows.is_empty(),
        "traversal-y name must be rejected before anything is persisted"
    );
}

#[tokio::test]
async fn list_bundled_empty_when_unset() {
    let deps = test_deps().await;
    let app = router(deps);
    let res = app.oneshot(get("/api/bundled")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn list_bundled_lists_wasm_stems() {
    let mut deps = test_deps().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("demo.wasm"), b"AGFzbQ").unwrap();
    deps.bundled_dir = Some(dir.path().to_path_buf());
    let app = router(deps);
    let res = app.oneshot(get("/api/bundled")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!([{"name": "demo"}]));
}

#[tokio::test]
async fn install_bundled_rejects_bad_name() {
    let deps = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/bundled/%2Fetc%2Fevil/install"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn install_bundled_conflict_when_unconfigured() {
    let deps = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/bundled/demo/install"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn static_assets_served_with_mime() {
    let deps = test_deps().await;
    let app = router(deps);
    for (path, mime) in [
        ("/", "text/html; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/style.css", "text/css; charset=utf-8"),
    ] {
        let res = app.clone().oneshot(get(path)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(
            res.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            mime,
            "{path}"
        );
        // The assets are baked into the binary: without no-cache, a browser
        // can heuristically keep serving the previous release's app.js after
        // an image update (observed live: new skill schema + stale JS saved
        // a prefix without recomposing the sensor name).
        assert_eq!(
            res.headers()
                .get(header::CACHE_CONTROL)
                .expect("static assets must send Cache-Control")
                .to_str()
                .unwrap(),
            "no-cache",
            "{path}"
        );
    }
    let missing = app.oneshot(get("/nope.png")).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------
// Real-`SkillsHandle` e2e tests.
//
// These build an actual `SkillRegistry` + `SkillDeps` (an unpolled MQTT
// `AsyncClient` — nothing connects until its event loop is polled, and the
// registry only publishes when a skill acts) so uploads/installs actually
// load a wasm module, and schema-driven behavior (masking, validation) runs
// against a real skill's `config_schema` export rather than a hand-rolled
// stand-in.
//
// Wasm fixtures come from `athena_voice_runtime::test_support`, which
// re-exports the paths the runtime crate's own `build.rs` already produces
// for its integration tests (`SMOKE_TEST_WASM` / `JEEDOM_TEST_WASM`, gated
// behind the `test-support` feature enabled in this crate's
// `[dev-dependencies]`). This crate does NOT read `skills/*.wasm` from disk:
// that directory is gitignored (each `skills-*/build.sh` populates it) and
// is absent in a fresh clone, whereas the runtime's build.rs unconditionally
// builds these two skills to `wasm32-wasip1` as part of `cargo test
// --workspace` / `cargo build --workspace`, so the fixture is always
// present without any extra setup step.
// ---------------------------------------------------------------------

fn test_skill_deps(store: Arc<dyn Store>) -> SkillDeps {
    let (mqtt, _event_loop) = rumqttc::AsyncClient::new(
        rumqttc::MqttOptions::new("admin-test", "127.0.0.1", 1883),
        16,
    );
    let (audio_tx, _rx) = broadcast::channel(8);
    SkillDeps {
        store,
        mqtt,
        tokio: tokio::runtime::Handle::current(),
        http: reqwest::Client::new(),
        locales: vec!["fr".into(), "en".into()],
        per_skill: HashMap::new(),
        event_tx: None,
        audio_event_tx: audio_tx,
    }
}

fn admin_deps(
    store: Arc<dyn Store>,
    registry: Arc<SkillRegistry>,
    dir: PathBuf,
    base_per_skill: HashMap<String, SkillConfig>,
    bundled_dir: Option<PathBuf>,
) -> AdminDeps {
    let skill_deps = test_skill_deps(store.clone());
    AdminDeps {
        store,
        skills: Some(SkillsHandle {
            registry,
            deps: skill_deps,
            dir,
        }),
        base_per_skill,
        bundled_dir,
    }
}

#[tokio::test]
async fn upload_installs_and_loads_a_real_skill() {
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let registry = deps.skills.as_ref().unwrap().registry.clone();
    let app = router(deps);

    let wasm = std::fs::read(athena_voice_runtime::test_support::SMOKE_TEST_WASM)
        .expect("SMOKE_TEST_WASM built by athena-voice-runtime's build.rs");
    let mut body = Vec::new();
    body.extend_from_slice(b"--BOUND\r\nContent-Disposition: form-data; name=\"file\"; filename=\"smoke-test.wasm\"\r\nContent-Type: application/wasm\r\n\r\n");
    body.extend_from_slice(&wasm);
    body.extend_from_slice(b"\r\n--BOUND--\r\n");
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/upload")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["ok"], true);
    assert!(out["reload_error"].is_null(), "reload failed: {out}");
    assert!(registry.skill_names().contains(&"smoke-test".to_string()));
    assert!(skills_dir.path().join("smoke-test.wasm").is_file());
}

#[tokio::test]
async fn install_bundled_installs_and_loads_a_real_skill() {
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    let bundled_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        athena_voice_runtime::test_support::SMOKE_TEST_WASM,
        bundled_dir.path().join("smoke-test.wasm"),
    )
    .expect("copy smoke-test.wasm into the bundled dir fixture");

    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        Some(bundled_dir.path().to_path_buf()),
    );
    let registry = deps.skills.as_ref().unwrap().registry.clone();
    let app = router(deps);

    let res = app
        .oneshot(post("/api/bundled/smoke-test/install"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["ok"], true);
    assert!(out["reload_error"].is_null(), "reload failed: {out}");
    assert!(
        skills_dir.path().join("smoke-test.wasm").is_file(),
        "bundled wasm must be copied into the skills dir"
    );
    assert!(registry.skill_names().contains(&"smoke-test".to_string()));
}

/// Loads the real `jeedom` skill (its `config_schema` export marks `api_key`
/// secret and `base_url` url-typed) into a fresh registry, so callers can
/// exercise masking / validation against an actual guest export rather than
/// a hand-rolled schema.
fn admin_deps_with_loaded_jeedom(
    store: Arc<dyn Store>,
    base_per_skill: HashMap<String, SkillConfig>,
) -> (AdminDeps, tempfile::TempDir) {
    let skills_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        athena_voice_runtime::test_support::JEEDOM_TEST_WASM,
        skills_dir.path().join("jeedom.wasm"),
    )
    .expect("copy jeedom.wasm into the skills dir fixture");

    let load_deps = test_skill_deps(store.clone());
    let registry = SkillRegistry::load_dir(skills_dir.path(), &load_deps)
        .expect("load jeedom.wasm into a fresh registry");

    let deps = admin_deps(
        store,
        Arc::new(registry),
        skills_dir.path().to_path_buf(),
        base_per_skill,
        None,
    );
    (deps, skills_dir)
}

#[tokio::test]
async fn skills_list_masks_schema_secret_sourced_only_from_toml() {
    // `api_key` has NO DB row here — its only value is the base TOML config
    // below. The registry's cached `config_schema` (from the real jeedom
    // wasm) marks `api_key` secret regardless, so GET /api/skills must still
    // mask it, and the raw TOML value must never appear in the response.
    let store = test_store().await;
    let toml_secret = "toml-only-s3cret-999";
    let mut base = HashMap::new();
    base.insert(
        "jeedom".to_string(),
        SkillConfig {
            config: HashMap::from([("api_key".to_string(), toml_secret.to_string())]),
            ..Default::default()
        },
    );
    let (deps, _skills_dir) = admin_deps_with_loaded_jeedom(store, base);
    let app = router(deps);

    let res = app.oneshot(get("/api/skills")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        !body_str.contains(toml_secret),
        "TOML-only secret value must never be echoed: {body_str}"
    );

    let body: serde_json::Value = serde_json::from_str(&body_str).unwrap();
    let jeedom = body
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "jeedom")
        .expect("jeedom listed");
    assert_eq!(jeedom["loaded"], true);
    assert_eq!(jeedom["config"]["api_key"]["kind"], "secret");
    assert_eq!(jeedom["config"]["api_key"]["set"], true);
}

#[tokio::test]
async fn put_config_rejects_invalid_schema_value_without_persisting() {
    // `base_url` is url-typed per the real jeedom schema; a scheme-less value
    // must be rejected by schema validation before anything is persisted.
    let store = test_store().await;
    let (deps, _skills_dir) = admin_deps_with_loaded_jeedom(store.clone(), HashMap::new());
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"values":{"base_url":"no-scheme"}}"#))
        .unwrap();
    let res = app.oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let rows = store.skill_settings_for("jeedom").await.unwrap();
    assert!(
        rows.is_empty(),
        "invalid schema value must be rejected before persisting"
    );
}

// ---------------------------------------------------------------------
// Fix 2: reload must not resurrect a disabled skill.
// ---------------------------------------------------------------------

#[tokio::test]
async fn put_config_does_not_resurrect_disabled_skill() {
    // jeedom.wasm sits on disk (as if previously installed) but the skill
    // was disabled before ever being loaded into THIS registry — e.g. the
    // process started with it already disabled. A config PUT must save the
    // value without reloading the skill: reload_skill's `wasm.is_file()`
    // check alone can't tell "disabled" from "installed and enabled", so it
    // must consult `skills_disabled()` before touching the registry.
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        athena_voice_runtime::test_support::JEEDOM_TEST_WASM,
        skills_dir.path().join("jeedom.wasm"),
    )
    .expect("copy jeedom.wasm into the skills dir fixture");
    store.skill_enabled_set("jeedom", false).await.unwrap();

    let deps = admin_deps(
        store.clone(),
        Arc::new(SkillRegistry::new()), // fresh, never loaded jeedom
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let registry = deps.skills.as_ref().unwrap().registry.clone();
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"values":{"base_url":"http://192.168.1.91"}}"#,
        ))
        .unwrap();
    let res = app.oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    assert!(
        !registry.skill_names().contains(&"jeedom".to_string()),
        "disabled skill must not be reloaded into the registry by a config write"
    );
    let rows = store.skill_settings_for("jeedom").await.unwrap();
    assert_eq!(
        rows[0].value, "http://192.168.1.91",
        "config is still saved"
    );
}

// ---------------------------------------------------------------------
// Fix 3: blank value must never overwrite a stored secret.
// ---------------------------------------------------------------------

#[tokio::test]
async fn put_config_blank_value_does_not_clobber_stored_secret_without_schema() {
    let deps = test_deps().await;
    let store = deps.store.clone();
    store
        .skill_setting_set("jeedom", "api_key", "real", true)
        .await
        .unwrap();
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"values":{"api_key":""}}"#))
        .unwrap();
    let res = app.oneshot(put).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let rows = store.skill_settings_for("jeedom").await.unwrap();
    let row = rows.iter().find(|r| r.key == "api_key").unwrap();
    assert_eq!(row.value, "real", "blank value must not clobber the secret");
    assert!(row.is_secret, "is_secret must remain true");
}

// ---------------------------------------------------------------------
// Fix 1: a failed upload/install must not brick the next restart.
// ---------------------------------------------------------------------

#[tokio::test]
async fn upload_quarantines_file_when_first_install_fails() {
    // No prior file for this name: a failed reload means the just-written
    // file must be deleted so a bad upload can't survive to the next
    // startup, where `load_dir` would otherwise trip over it again.
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let app = router(deps);

    let mut body = Vec::new();
    body.extend_from_slice(b"--BOUND\r\nContent-Disposition: form-data; name=\"file\"; filename=\"badskill.wasm\"\r\nContent-Type: application/wasm\r\n\r\n");
    body.extend_from_slice(b"not really wasm");
    body.extend_from_slice(b"\r\n--BOUND--\r\n");
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/upload")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !out["reload_error"].is_null(),
        "expected a reload error: {out}"
    );
    assert_eq!(out["removed"], true);
    assert!(
        !skills_dir.path().join("badskill.wasm").exists(),
        "failed first install must not leave a file behind for the next startup"
    );
}

#[tokio::test]
async fn upload_keeps_file_when_overwrite_of_working_skill_fails() {
    // A working skill is already installed; a failed re-upload must NOT
    // delete the file out from under the running process — only the
    // never-previously-loaded case gets quarantined.
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        athena_voice_runtime::test_support::SMOKE_TEST_WASM,
        skills_dir.path().join("smoke-test.wasm"),
    )
    .unwrap();
    let load_deps = test_skill_deps(store.clone());
    let registry = SkillRegistry::load_dir(skills_dir.path(), &load_deps).unwrap();
    let deps = admin_deps(
        store,
        Arc::new(registry),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    );
    let app = router(deps);

    let mut body = Vec::new();
    body.extend_from_slice(b"--BOUND\r\nContent-Disposition: form-data; name=\"file\"; filename=\"smoke-test.wasm\"\r\nContent-Type: application/wasm\r\n\r\n");
    body.extend_from_slice(b"not really wasm");
    body.extend_from_slice(b"\r\n--BOUND--\r\n");
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/upload")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=BOUND")
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !out["reload_error"].is_null(),
        "expected a reload error: {out}"
    );
    assert_eq!(out["removed"], false);
    assert!(
        skills_dir.path().join("smoke-test.wasm").exists(),
        "overwrite of a previously-working skill must keep the new (broken) file, not delete it"
    );
}

#[tokio::test]
async fn install_bundled_quarantines_file_when_first_install_fails() {
    // Same quarantine contract for the `install_bundled` path: a "bundled"
    // wasm that's actually garbage (e.g. a corrupted asset) must not leave
    // a file behind that the next startup would try (and fail) to load.
    let store = test_store().await;
    let skills_dir = tempfile::tempdir().unwrap();
    let bundled_dir = tempfile::tempdir().unwrap();
    std::fs::write(bundled_dir.path().join("badskill.wasm"), b"not really wasm").unwrap();
    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        Some(bundled_dir.path().to_path_buf()),
    );
    let app = router(deps);

    let res = app
        .oneshot(post("/api/bundled/badskill/install"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !out["reload_error"].is_null(),
        "expected a reload error: {out}"
    );
    assert_eq!(out["removed"], true);
    assert!(
        !skills_dir.path().join("badskill.wasm").exists(),
        "failed first install must not leave a file behind for the next startup"
    );
}

// Jeedom connection test endpoint tests
async fn deps_with_jeedom_config(base_url: &str) -> AdminDeps {
    let deps = test_deps().await;
    deps.store
        .skill_setting_set("jeedom", "base_url", base_url, false)
        .await
        .unwrap();
    deps.store
        .skill_setting_set("jeedom", "api_key", "sekret-key-123", true)
        .await
        .unwrap();
    deps
}

#[tokio::test]
async fn jeedom_test_reports_ok_via_object_list() {
    // The documented lightweight authenticated call is `type=object`
    // (https://doc.jeedom.com/fr_FR/core/4.5/api_http) — `type=version`
    // does NOT exist in the HTTP API (Jeedom 4.5 returns an empty body,
    // which the old heuristic misread as a bad key).
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/core/api/jeeApi.php"))
        .and(wiremock::matchers::query_param("type", "object"))
        .and(wiremock::matchers::query_param("apikey", "sekret-key-123"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(r#"[{"id":"1","name":"Salon"},{"id":"2","name":"Garage"}]"#),
        )
        .mount(&server)
        .await;
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("sekret-key-123"),
        "api key must never be echoed"
    );
}

#[tokio::test]
async fn jeedom_test_classifies_failures() {
    // unauthorized: Jeedom answers 200 with an error sentence, not a version.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("Clé API non valide"))
        .mount(&server)
        .await;
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .clone()
        .oneshot(post("/api/skills/jeedom/test"))
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "unauthorized");

    // An empty 200 body (e.g. an unknown `type=` call) is not proof of a
    // valid key either — must not report ok.
    let empty = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(""))
        .mount(&empty)
        .await;
    let deps = deps_with_jeedom_config(&empty.uri()).await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test")).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "unauthorized");

    // unreachable: explicit unreachable port.
    drop(server);
    let deps = deps_with_jeedom_config("http://127.0.0.1:1").await;
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test")).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "unreachable");
}

#[tokio::test]
async fn jeedom_test_unconfigured() {
    let deps = test_deps().await; // no jeedom config at all
    let app = router(deps);
    let res = app.oneshot(post("/api/skills/jeedom/test")).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "unconfigured");
}

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
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/discover"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .unwrap();
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
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/discover"))
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "bad_response");
}

#[tokio::test]
async fn jeedom_discover_aborts_on_oversized_response() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::query_param("type", "fullData"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_bytes(vec![b'['; 5 * 1024 * 1024]), // 5 MiB, exceeds 4 MiB cap
        )
        .mount(&server)
        .await;
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/discover"))
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), 1 << 23)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["status"], "bad_response",
        "oversized response must be rejected"
    );
}

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
                r#"[{"name":"température salon","id":142,"unit":"°C","room":"salon"},
                    {"name":"température d'alicia","id":7,"room":"alicia","prefix":"d'"}]"#
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
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
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
}

#[tokio::test]
async fn jeedom_phrases_empty_when_skill_not_loaded() {
    // No skill runtime at all (skills: None) → empty phrases, same shape.
    let deps = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(get("/api/skills/jeedom/phrases"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
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
    let res = app
        .oneshot(get("/api/skills/jeedom/phrases"))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!({ "phrases": [] }));
}

// Jeedom single-sensor read endpoint tests

async fn read_body(res: axum::response::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
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

#[tokio::test]
async fn jeedom_discover_prunes_empty_equipment_and_rooms() {
    // Fixture: Salon has one eqLogic with only action cmds (pruned).
    // Kitchen has one eqLogic with an info cmd and an action cmd.
    // Bathroom room has eqLogics but all cmds are actions or unnamed (room pruned).
    let pruning_fixture = r#"[
  { "name": "Salon", "eqLogics": [
    { "name": "Commutateur", "cmds": [
      { "id": 1, "name": "Allumer", "type": "action", "subType": "other" },
      { "id": 2, "name": "Éteindre", "type": "action", "subType": "other" }
    ] }
  ] },
  { "name": "Cuisine", "eqLogics": [
    { "name": "Capteur", "cmds": [
      { "id": 10, "name": "Température", "type": "info", "subType": "numeric", "unite": "°C" },
      { "id": 11, "name": "Allumer", "type": "action", "subType": "other" }
    ] }
  ] },
  { "name": "Salle de bain", "eqLogics": [
    { "name": "Ventilatrice", "cmds": [
      { "id": 20, "name": "Activer", "type": "action", "subType": "other" },
      { "id": 21, "name": "", "type": "info", "subType": "numeric" }
    ] }
  ] }
]"#;

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::query_param("type", "fullData"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(pruning_fixture))
        .mount(&server)
        .await;
    let deps = deps_with_jeedom_config(&server.uri()).await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/skills/jeedom/discover"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["status"], "ok");

    let rooms = body["rooms"].as_array().unwrap();
    assert_eq!(
        rooms.len(),
        1,
        "only Cuisine survives (Salon + Salle de bain have no info cmds)"
    );
    assert_eq!(rooms[0]["name"], "Cuisine");

    let equipments = rooms[0]["equipments"].as_array().unwrap();
    assert_eq!(equipments.len(), 1);
    assert_eq!(equipments[0]["name"], "Capteur");

    let cmds = equipments[0]["cmds"].as_array().unwrap();
    assert_eq!(cmds.len(), 1, "action cmd filtered out");
    assert_eq!(cmds[0]["id"], 10);
    assert_eq!(cmds[0]["name"], "Température");
}
