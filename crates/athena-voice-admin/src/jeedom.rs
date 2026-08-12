//! Jeedom-specific admin endpoints: connection test and sensor discovery.
//! Host-side by design (spec 2026-07-26): the box is called with the SAVED
//! merged config, and the API key never travels to the browser.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
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

/// One paired on/off device discovered on an equipment.
#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct DiscoveredAction {
    pub(crate) on_id: u64,
    pub(crate) off_id: u64,
}

/// A raw action-type command as seen in fullData, before pairing.
pub(crate) struct ActionCmd {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) generic: Option<String>,
}

/// On/off name vocabulary, index-aligned: `NAME_ON[i]` pairs `NAME_OFF[i]`.
const NAME_ON: [&str; 4] = ["on", "allumer", "marche", "activer"];
const NAME_OFF: [&str; 4] = ["off", "éteindre", "arrêt", "désactiver"];

/// Pairs raw action commands into on/off devices: `generic_type`
/// (`FOO_ON`/`FOO_OFF` with the same prefix) first, then case-insensitive
/// name vocabulary. Unpaired commands are ignored (dimmers, scenarios and
/// friends are out of scope).
pub(crate) fn pair_actions(cmds: &[ActionCmd]) -> Vec<DiscoveredAction> {
    let mut out = Vec::new();
    let mut used: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // Pass 1: generic_type prefixes.
    for c in cmds {
        let Some(prefix) = c.generic.as_deref().and_then(|g| g.strip_suffix("_ON")) else {
            continue;
        };
        let off_generic = format!("{prefix}_OFF");
        if let Some(off) = cmds
            .iter()
            .find(|o| o.generic.as_deref() == Some(off_generic.as_str()) && !used.contains(&o.id))
        {
            if used.insert(c.id) && used.insert(off.id) {
                out.push(DiscoveredAction {
                    on_id: c.id,
                    off_id: off.id,
                });
            }
        }
    }
    // Pass 2: name vocabulary, index-aligned (On↔Off, Allumer↔Éteindre, …).
    for (i, on_name) in NAME_ON.iter().enumerate() {
        let on = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == *on_name);
        let off = cmds
            .iter()
            .find(|c| !used.contains(&c.id) && c.name.to_lowercase() == NAME_OFF[i]);
        if let (Some(on), Some(off)) = (on, off) {
            used.insert(on.id);
            used.insert(off.id);
            out.push(DiscoveredAction {
                on_id: on.id,
                off_id: off.id,
            });
        }
    }
    out
}

#[derive(serde::Serialize)]
pub(crate) struct DiscoveredEquipment {
    name: String,
    cmds: Vec<DiscoveredCmd>,
    actions: Vec<DiscoveredAction>,
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
            let mut action_cmds: Vec<ActionCmd> = Vec::new();
            for cmd in cmd_iter {
                let cmd_type = cmd.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let name = cmd.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let Some(id) = cmd.get("id").and_then(cmd_id) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                if cmd_type == "action" {
                    action_cmds.push(ActionCmd {
                        id,
                        name: name.to_string(),
                        generic: cmd
                            .get("generic_type")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                    });
                    continue;
                }
                if cmd_type != "info" {
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
            let actions = pair_actions(&action_cmds);
            if !cmds.is_empty() || !actions.is_empty() {
                equipments.push(DiscoveredEquipment {
                    name: eq_name,
                    cmds,
                    actions,
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

/// Host-side single-sensor read with the SAVED merged config — same pattern
/// as test/discover: the API key never travels to the browser, and the URL
/// (which embeds the key as a query param) is never logged.
pub(crate) async fn read_one(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let Some(cfg) = resolved_config(&state).await else {
        return status_json("unconfigured");
    };
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={}&type=cmd&id={id}",
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
    // A bad key gets Jeedom's prose sentence (HTTP 200) — not valid JSON, so
    // it lands on bad_response; test/discover already classify authorization.
    let Ok(raw) = serde_json::from_str::<serde_json::Value>(&body) else {
        return status_json("bad_response");
    };
    // Same tolerance as the skill's read path: bare scalar, string-wrapped
    // number, or a `{"value": …}` envelope.
    let value = raw.get("value").cloned().unwrap_or(raw);
    let spoken = match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    Json(serde_json::json!({ "status": "ok", "value": spoken })).into_response()
}

/// Every configured locale's pattern rules from the LOADED jeedom skill, via
/// the registry's rule cache — the UI shows the truth about what can be said
/// instead of re-deriving phrase templates in JS. Reflects the SAVED config:
/// a config save reloads the plugin, which repopulates the cache.
pub(crate) async fn phrases(State(state): State<AppState>) -> Response {
    let mut out: Vec<serde_json::Value> = Vec::new();
    if let Some(handle) = &state.skills {
        for locale in &handle.deps.locales {
            for rule in handle
                .registry
                .skill_rules("jeedom", locale)
                .unwrap_or_default()
            {
                out.push(serde_json::json!({
                    "intent": rule.intent,
                    "locale": locale,
                    "phrases": rule.phrases,
                }));
            }
        }
    }
    Json(serde_json::json!({ "phrases": out })).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ac(id: u64, name: &str, generic: Option<&str>) -> ActionCmd {
        ActionCmd {
            id,
            name: name.into(),
            generic: generic.map(String::from),
        }
    }

    #[test]
    fn pairs_by_generic_type_first() {
        let cmds = vec![
            ac(124, "Bouton On", Some("LIGHT_ON")),
            ac(125, "Bouton Off", Some("LIGHT_OFF")),
            ac(200, "Refresh", Some("DONT_CARE")),
        ];
        assert_eq!(
            pair_actions(&cmds),
            vec![DiscoveredAction {
                on_id: 124,
                off_id: 125
            }]
        );
    }

    #[test]
    fn pairs_by_french_and_english_names() {
        let cmds = vec![
            ac(7, "Allumer", None),
            ac(8, "Éteindre", None),
            ac(30, "On", None),
            ac(31, "Off", None),
            ac(90, "Rafraîchir", None),
        ];
        let pairs = pair_actions(&cmds);
        assert!(pairs.contains(&DiscoveredAction {
            on_id: 7,
            off_id: 8
        }));
        assert!(pairs.contains(&DiscoveredAction {
            on_id: 30,
            off_id: 31
        }));
        assert_eq!(pairs.len(), 2, "unpaired leftovers are ignored");
    }

    #[test]
    fn fulldata_carries_paired_actions_per_equipment() {
        let raw = serde_json::json!([{
            "name": "Salon",
            "eqLogics": [{
                "name": "Lampe",
                "cmds": [
                    {"id": 1, "type": "info", "subType": "numeric", "name": "Puissance"},
                    {"id": 124, "type": "action", "subType": "other", "name": "On"},
                    {"id": 125, "type": "action", "subType": "other", "name": "Off"}
                ]
            }]
        }]);
        let rooms = parse_fulldata(&raw).unwrap();
        let eq = &rooms[0].equipments[0];
        assert_eq!(eq.cmds.len(), 1, "info commands unchanged");
        assert_eq!(
            eq.actions,
            vec![DiscoveredAction {
                on_id: 124,
                off_id: 125
            }]
        );
    }

    #[test]
    fn equipment_with_only_actions_still_surfaces() {
        // An on/off-only device has no info commands; it must not be
        // dropped by the "no cmds → skip equipment" guard.
        let raw = serde_json::json!([{
            "name": "Garage",
            "eqLogics": [{
                "name": "Portail",
                "cmds": [
                    {"id": 30, "type": "action", "subType": "other", "name": "Marche"},
                    {"id": 31, "type": "action", "subType": "other", "name": "Arrêt"}
                ]
            }]
        }]);
        let rooms = parse_fulldata(&raw).unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].equipments[0].actions.len(), 1);
    }
}
