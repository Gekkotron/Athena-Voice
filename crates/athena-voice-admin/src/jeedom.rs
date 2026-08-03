//! Jeedom-specific admin endpoints: connection test and sensor discovery.
//! Host-side by design (spec 2026-07-26): the box is called with the SAVED
//! merged config, and the API key never travels to the browser.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;

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

pub(crate) async fn test_connection(State(state): State<AppState>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    // `type=object` is the lightest documented authenticated call in the
    // Jeedom HTTP API (https://doc.jeedom.com/fr_FR/core/4.5/api_http):
    // a valid key returns the object list as a JSON array; a bad key gets
    // Jeedom's prose error (still HTTP 200). `type=version` does NOT exist
    // in this API — Jeedom 4.5 answers it with an empty body.
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=object",
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
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(serde_json::Value::Array(_)) => status_json("ok"),
        _ => status_json("unauthorized"),
    }
}

const FULLDATA_CAP_BYTES: usize = 4 * 1024 * 1024;

/// Jeedom's `id` arrives as a number or a numeric string depending on
/// version/plugin; normalize to u64.
fn cmd_id(v: &serde_json::Value) -> Option<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[derive(serde::Serialize)]
pub(crate) struct DiscoveredCmd {
    id: u64,
    name: String,
    subtype: String,
    unit: Option<String>,
    on_label: Option<String>,
    off_label: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct DiscoveredEquipment {
    name: String,
    cmds: Vec<DiscoveredCmd>,
}

#[derive(serde::Serialize)]
pub(crate) struct DiscoveredRoom {
    name: String,
    equipments: Vec<DiscoveredEquipment>,
}

/// Defensive walk of the fullData array: every field is optional, anything
/// malformed is skipped rather than fatal, and only `type == "info"`
/// commands with a non-empty name survive.
fn parse_fulldata(raw: &serde_json::Value) -> Option<Vec<DiscoveredRoom>> {
    let objects = raw.as_array()?;
    let mut rooms = Vec::new();
    for obj in objects {
        let room_name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut equipments = Vec::new();
        let eq_iter = obj
            .get("eqLogics")
            .and_then(|v| v.as_array())
            .map_or(&[][..], |a| a.as_slice());
        for eq in eq_iter {
            let eq_name = eq
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut cmds = Vec::new();
            let cmd_iter = eq
                .get("cmds")
                .and_then(|v| v.as_array())
                .map_or(&[][..], |a| a.as_slice());
            for cmd in cmd_iter {
                if cmd.get("type").and_then(|v| v.as_str()) != Some("info") {
                    continue;
                }
                let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let Some(id) = cmd.get("id").and_then(cmd_id) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let display = cmd.get("display");
                let label = |key: &str| {
                    display
                        .and_then(|d| d.get(key))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                };
                cmds.push(DiscoveredCmd {
                    id,
                    name: name.to_string(),
                    subtype: cmd
                        .get("subType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("numeric")
                        .to_string(),
                    unit: cmd
                        .get("unite")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                    on_label: label("on_label"),
                    off_label: label("off_label"),
                });
            }
            if !cmds.is_empty() {
                equipments.push(DiscoveredEquipment {
                    name: eq_name,
                    cmds,
                });
            }
        }
        if !equipments.is_empty() {
            rooms.push(DiscoveredRoom {
                name: room_name,
                equipments,
            });
        }
    }
    Some(rooms)
}

pub(crate) async fn discover(State(state): State<AppState>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=fullData",
        cfg.base_url, cfg.api_key
    );
    let Ok(resp) = state
        .http
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    else {
        return status_json("unreachable");
    };
    if !resp.status().is_success() {
        return status_json("bad_response");
    }

    // Stream the response body with early abort if size exceeds cap.
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return status_json("bad_response");
        };
        if buf.len() + chunk.len() > FULLDATA_CAP_BYTES {
            return status_json("bad_response");
        }
        buf.extend_from_slice(&chunk);
    }

    let Ok(raw) = serde_json::from_slice::<serde_json::Value>(&buf) else {
        return status_json("bad_response");
    };
    match parse_fulldata(&raw) {
        Some(rooms) => Json(serde_json::json!({ "status": "ok", "rooms": rooms })).into_response(),
        None => status_json("bad_response"),
    }
}
