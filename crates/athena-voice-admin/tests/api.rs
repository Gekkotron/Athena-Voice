use std::collections::HashMap;
use std::sync::Arc;

use athena_voice_admin::{AdminDeps, auth, router};
use athena_voice_storage::{SqliteStore, Store};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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
