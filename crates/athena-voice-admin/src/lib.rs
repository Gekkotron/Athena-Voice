#![deny(warnings)]
//! Admin web interface: token-protected JSON API + embedded static UI.

pub mod auth;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use athena_voice_runtime::SkillsHandle;
use athena_voice_runtime::wasm::registry::SkillConfig;
use athena_voice_storage::Store;

pub struct AdminDeps {
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    /// TOML-derived per-skill config — the merge base; DB rows override it.
    pub base_per_skill: HashMap<String, SkillConfig>,
    pub token_hash: String,
    pub bundled_dir: Option<PathBuf>,
}

// `store`, `base_per_skill`, and `bundled_dir` aren't read by any handler
// yet — Task 7 only wires `/api/status`. Tasks 8-10 add the skills
// list/config/enable/upload/bundled routes that read them.
#[derive(Clone)]
pub(crate) struct AppState {
    #[allow(dead_code)]
    pub store: Arc<dyn Store>,
    pub skills: Option<SkillsHandle>,
    #[allow(dead_code)]
    pub base_per_skill: Arc<HashMap<String, SkillConfig>>,
    pub token_hash: Arc<String>,
    #[allow(dead_code)]
    pub bundled_dir: Option<PathBuf>,
}

pub fn router(deps: AdminDeps) -> Router {
    let state = AppState {
        store: deps.store,
        skills: deps.skills,
        base_per_skill: Arc::new(deps.base_per_skill),
        token_hash: Arc::new(deps.token_hash),
        bundled_dir: deps.bundled_dir,
    };
    // The api sub-router is fully stated (Router<()>) before nesting; the
    // outer router stays stateless — the asset handler needs no state.
    let api = Router::new()
        .route("/status", get(status))
        // Task 8+ add: skills list/config/enable/upload/bundled routes here.
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
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

/// Placeholder until Task 12 embeds the real UI: serve a stub index so the
/// root URL is 200 from day one.
async fn static_asset() -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><title>Athena-Voice</title><p>Admin UI comes in a later task.</p>",
    )
        .into_response()
}
