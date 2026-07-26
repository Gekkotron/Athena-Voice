#![deny(warnings)]
//! Admin web interface: token-protected JSON API + embedded static UI.

pub(crate) mod api;
pub mod auth;
pub(crate) mod jeedom;
pub(crate) mod validate;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use include_dir::{Dir, include_dir};

use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::SkillConfig;
use athena_voice_storage::Store;

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/static");

pub struct AdminDeps {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    /// TOML-derived per-skill config — the merge base; DB rows override it.
    pub base_per_skill: HashMap<String, SkillConfig>,
    pub token_hash: String,
    pub bundled_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    pub base_per_skill: Arc<HashMap<String, SkillConfig>>,
    pub token_hash: Arc<String>,
    pub bundled_dir: Option<PathBuf>,
    pub http: reqwest::Client,
}

pub fn router(deps: AdminDeps) -> Router {
    let state = AppState {
        store: deps.store,
        skills: deps.skills,
        base_per_skill: Arc::new(deps.base_per_skill),
        token_hash: Arc::new(deps.token_hash),
        bundled_dir: deps.bundled_dir,
        http: reqwest::Client::new(),
    };
    // The api sub-router is fully stated (Router<()>) before nesting; the
    // outer router stays stateless — the asset handler needs no state.
    let api = Router::new()
        .route("/status", get(status))
        .route("/skills", get(api::list_skills))
        .route("/skills/{name}/config", axum::routing::put(api::put_config))
        .route(
            "/skills/{name}/enable",
            axum::routing::post(api::enable_skill),
        )
        .route(
            "/skills/{name}/disable",
            axum::routing::post(api::disable_skill),
        )
        .route(
            "/skills/jeedom/test",
            axum::routing::post(jeedom::test_connection),
        )
        .route("/skills/upload", axum::routing::post(api::upload_skill))
        .route("/bundled", get(api::list_bundled))
        .route(
            "/bundled/{name}/install",
            axum::routing::post(api::install_bundled),
        )
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state);
    Router::new().nest("/api", api).fallback(get(static_asset))
}

/// Bind and serve forever (spawned as a background task by `serve`).
pub async fn serve(addr: SocketAddr, deps: AdminDeps) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "admin UI listening");
    axum::serve(listener, router(deps)).await?;
    Ok(())
}

async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| auth::verify(&state.token_hash, t));
    if ok {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn status(State(state): State<AppState>) -> Response {
    let loaded = state
        .skills
        .as_ref()
        .map_or(0, |h| h.registry.skill_names().len());
    axum::Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "skills_loaded": loaded,
    }))
    .into_response()
}

async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = ASSETS.get_file(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
}
