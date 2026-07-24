use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use athena_voice_admin::{AdminDeps, auth, router};
use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::{SkillConfig, SkillDeps, SkillRegistry};
use athena_voice_storage::{SqliteStore, Store};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tokio::sync::broadcast;
use tower::ServiceExt; // oneshot

async fn test_deps() -> (AdminDeps, String) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let token = auth::ensure_token(&store)
        .await
        .unwrap()
        .expect("first run yields a token");
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
    let bad = app
        .clone()
        .oneshot(get("/api/status", Some("wrong")))
        .await
        .unwrap();
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
    let res = app.oneshot(get("/api/skills", Some(&token))).await.unwrap();
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
    let (deps, token) = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);

    // No schema (skill not loaded) → free-form values accepted.
    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
    let (deps, token) = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
    let res = app
        .clone()
        .oneshot(post("/api/skills/timer/disable", &token))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        store.skills_disabled().await.unwrap(),
        vec!["timer".to_string()]
    );
    let res = app
        .oneshot(post("/api/skills/timer/enable", &token))
        .await
        .unwrap();
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

#[tokio::test]
async fn put_config_rejects_traversal_name() {
    // Percent-encoded `/` inside the path segment: axum's router matches on
    // the raw (still-encoded) path, so this stays a single `{name}` segment,
    // but the `Path` extractor hands the handler the decoded `/etc/evil` —
    // which must be rejected by `valid_skill_name` before it ever reaches a
    // store write or `dir.join(...)`.
    let (deps, token) = test_deps().await;
    let store = deps.store.clone();
    let app = router(deps);
    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/%2Fetc%2Fevil/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
    let (deps, token) = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(get("/api/bundled", Some(&token)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn list_bundled_lists_wasm_stems() {
    let (mut deps, token) = test_deps().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("demo.wasm"), b"AGFzbQ").unwrap();
    deps.bundled_dir = Some(dir.path().to_path_buf());
    let app = router(deps);
    let res = app
        .oneshot(get("/api/bundled", Some(&token)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!([{"name": "demo"}]));
}

#[tokio::test]
async fn install_bundled_rejects_bad_name() {
    let (deps, token) = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/bundled/%2Fetc%2Fevil/install", &token))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn install_bundled_conflict_when_unconfigured() {
    let (deps, token) = test_deps().await;
    let app = router(deps);
    let res = app
        .oneshot(post("/api/bundled/demo/install", &token))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

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
            res.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            mime,
            "{path}"
        );
    }
    let missing = app.oneshot(get("/nope.png", None)).await.unwrap();
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

async fn store_with_token() -> (Arc<dyn Store>, String) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("sqlite::memory:").await.unwrap());
    let token = auth::ensure_token(&store)
        .await
        .unwrap()
        .expect("first run yields a token");
    (store, token)
}

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

async fn admin_deps(
    store: Arc<dyn Store>,
    registry: Arc<SkillRegistry>,
    dir: PathBuf,
    base_per_skill: HashMap<String, SkillConfig>,
    bundled_dir: Option<PathBuf>,
) -> AdminDeps {
    let skill_deps = test_skill_deps(store.clone());
    let hash = store.admin_token_hash().await.unwrap().unwrap();
    AdminDeps {
        store,
        skills: Some(SkillsHandle {
            registry,
            deps: skill_deps,
            dir,
        }),
        base_per_skill,
        token_hash: hash,
        bundled_dir,
    }
}

#[tokio::test]
async fn upload_installs_and_loads_a_real_skill() {
    let (store, token) = store_with_token().await;
    let skills_dir = tempfile::tempdir().unwrap();
    let deps = admin_deps(
        store,
        Arc::new(SkillRegistry::new()),
        skills_dir.path().to_path_buf(),
        HashMap::new(),
        None,
    )
    .await;
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
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
    let (store, token) = store_with_token().await;
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
    )
    .await;
    let registry = deps.skills.as_ref().unwrap().registry.clone();
    let app = router(deps);

    let res = app
        .oneshot(post("/api/bundled/smoke-test/install", &token))
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
async fn admin_deps_with_loaded_jeedom(
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
    )
    .await;
    (deps, skills_dir)
}

#[tokio::test]
async fn skills_list_masks_schema_secret_sourced_only_from_toml() {
    // `api_key` has NO DB row here — its only value is the base TOML config
    // below. The registry's cached `config_schema` (from the real jeedom
    // wasm) marks `api_key` secret regardless, so GET /api/skills must still
    // mask it, and the raw TOML value must never appear in the response.
    let (store, token) = store_with_token().await;
    let toml_secret = "toml-only-s3cret-999";
    let mut base = HashMap::new();
    base.insert(
        "jeedom".to_string(),
        SkillConfig {
            config: HashMap::from([("api_key".to_string(), toml_secret.to_string())]),
            ..Default::default()
        },
    );
    let (deps, _skills_dir) = admin_deps_with_loaded_jeedom(store, base).await;
    let app = router(deps);

    let res = app.oneshot(get("/api/skills", Some(&token))).await.unwrap();
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
    let (store, token) = store_with_token().await;
    let (deps, _skills_dir) = admin_deps_with_loaded_jeedom(store.clone(), HashMap::new()).await;
    let app = router(deps);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/skills/jeedom/config")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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
