#![deny(warnings)]
//! Admin web interface: JSON API + embedded static UI.

pub(crate) mod api;
pub(crate) mod jeedom;
pub(crate) mod validate;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
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
    pub bundled_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    pub base_per_skill: Arc<HashMap<String, SkillConfig>>,
    pub bundled_dir: Option<PathBuf>,
    pub http: reqwest::Client,
}

pub fn router(deps: AdminDeps) -> Router {
    let state = AppState {
        store: deps.store,
        skills: deps.skills,
        base_per_skill: Arc::new(deps.base_per_skill),
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
        .route(
            "/skills/jeedom/discover",
            axum::routing::post(jeedom::discover),
        )
        .route(
            "/skills/jeedom/read/{id}",
            axum::routing::post(jeedom::read_one),
        )
        .route("/skills/jeedom/phrases", get(jeedom::phrases))
        .route("/skills/upload", axum::routing::post(api::upload_skill))
        .route("/bundled", get(api::list_bundled))
        .route(
            "/bundled/{name}/install",
            axum::routing::post(api::install_bundled),
        )
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
    // no-cache = revalidate on every use. The assets are embedded in the
    // binary, so freshness otherwise changes only on process upgrade — and
    // heuristic browser caching was observed serving a previous release's
    // app.js against a newer skill schema.
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        file.contents(),
    )
        .into_response()
}
