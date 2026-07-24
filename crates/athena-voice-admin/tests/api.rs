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
