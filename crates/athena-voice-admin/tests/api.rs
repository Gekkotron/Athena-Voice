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
