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
    let base = state
        .base_per_skill
        .get("jeedom")
        .cloned()
        .unwrap_or_default();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let merged = apply_settings(&base, &pairs);
    let base_url = merged
        .config
        .get("base_url")
        .filter(|s| !s.is_empty())?
        .clone();
    let api_key = merged
        .config
        .get("api_key")
        .filter(|s| !s.is_empty())?
        .clone();
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
    let Ok(resp) = state
        .http
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    else {
        return status_json("unreachable");
    };
    if !resp.status().is_success() {
        return status_json("bad_response");
    }
    let Ok(body) = resp.text().await else {
        return status_json("bad_response");
    };
    if looks_like_version(&body) {
        Json(serde_json::json!({ "status": "ok", "version": body.trim() })).into_response()
    } else {
        status_json("unauthorized")
    }
}
