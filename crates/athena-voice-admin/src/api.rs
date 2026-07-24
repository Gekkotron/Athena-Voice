//! JSON handlers for the admin API.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use athena_voice_runtime::wasm::settings::{HTTP_ALLOWLIST_KEY, apply_settings};
use athena_voice_skill_sdk::ConfigSchema;
use athena_voice_storage::models::SkillSettingRow;

use crate::AppState;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum ConfigValue {
    Plain { value: String },
    Secret { set: bool },
}

#[derive(Serialize)]
pub(crate) struct SkillInfo {
    name: String,
    loaded: bool,
    enabled: bool,
    schema: Option<ConfigSchema>,
    config: BTreeMap<String, ConfigValue>,
    http_allowlist: Vec<String>,
}

pub(crate) fn internal_error(e: impl std::fmt::Display) -> Response {
    tracing::error!(error = %e, "admin api error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": e.to_string()})),
    )
        .into_response()
}

/// Union of: base TOML keys, loaded registry names, wasm files on disk,
/// skills with DB settings rows, and skills with a disabled-flag row.
/// Built entirely from data already fetched by `list_skills` — no store
/// calls here.
fn known_skill_names(
    state: &AppState,
    rows_by_skill: &BTreeMap<String, Vec<SkillSettingRow>>,
    disabled: &BTreeSet<String>,
    registry_names: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut names: BTreeSet<String> = state.base_per_skill.keys().cloned().collect();
    names.extend(registry_names.iter().cloned());
    names.extend(rows_by_skill.keys().cloned());
    names.extend(disabled.iter().cloned());
    if let Some(handle) = &state.skills
        && let Ok(entries) = std::fs::read_dir(&handle.dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.insert(stem.to_string());
            }
        }
    }
    names
}

/// Build one skill's response entry from data the caller already fetched —
/// no store calls happen in here, so this can be called once per skill
/// without any extra round trips.
pub(crate) fn skill_info(
    state: &AppState,
    name: &str,
    rows: Vec<SkillSettingRow>,
    disabled: &BTreeSet<String>,
    registry_names: &BTreeSet<String>,
) -> SkillInfo {
    let secret_keys: BTreeSet<String> = rows
        .iter()
        .filter(|r| r.is_secret)
        .map(|r| r.key.clone())
        .collect();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let base = state.base_per_skill.get(name).cloned().unwrap_or_default();
    let merged = apply_settings(&base, &pairs);

    let loaded = registry_names.contains(name);
    let schema = state
        .skills
        .as_ref()
        .and_then(|h| h.registry.config_schema(name));
    // Schema marks secrets too, even when the value still comes from TOML.
    let schema_secret_keys: BTreeSet<String> = schema
        .as_ref()
        .map(|s| {
            s.fields
                .iter()
                .filter(|f| f.is_secret())
                .map(|f| f.key.clone())
                .collect()
        })
        .unwrap_or_default();

    let config = merged
        .config
        .iter()
        .filter(|(k, _)| k.as_str() != HTTP_ALLOWLIST_KEY)
        .map(|(k, v)| {
            let value = if secret_keys.contains(k) || schema_secret_keys.contains(k) {
                ConfigValue::Secret { set: !v.is_empty() }
            } else {
                ConfigValue::Plain { value: v.clone() }
            };
            (k.clone(), value)
        })
        .collect();

    SkillInfo {
        name: name.to_string(),
        loaded,
        enabled: !disabled.contains(name),
        schema,
        config,
        http_allowlist: merged.http_allowlist,
    }
}

pub(crate) async fn list_skills(State(state): State<AppState>) -> Response {
    // Exactly two store round trips total, no matter how many skills exist.
    let all_rows = match state.store.skill_settings_all().await {
        Ok(rows) => rows,
        Err(e) => return internal_error(e),
    };
    let disabled: BTreeSet<String> = match state.store.skills_disabled().await {
        Ok(names) => names.into_iter().collect(),
        Err(e) => return internal_error(e),
    };

    let mut rows_by_skill: BTreeMap<String, Vec<SkillSettingRow>> = BTreeMap::new();
    for row in all_rows {
        rows_by_skill
            .entry(row.skill.clone())
            .or_default()
            .push(row);
    }

    // Computed once (Minor 1) rather than per skill.
    let registry_names: BTreeSet<String> = state
        .skills
        .as_ref()
        .map(|h| h.registry.skill_names().into_iter().collect())
        .unwrap_or_default();

    let names = known_skill_names(&state, &rows_by_skill, &disabled, &registry_names);
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let rows = rows_by_skill.remove(&name).unwrap_or_default();
        out.push(skill_info(&state, &name, rows, &disabled, &registry_names));
    }
    Json(out).into_response()
}

#[derive(serde::Deserialize)]
pub(crate) struct ConfigWrite {
    values: HashMap<String, String>,
}

pub(crate) async fn put_config(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ConfigWrite>,
) -> Response {
    let schema = state
        .skills
        .as_ref()
        .and_then(|h| h.registry.config_schema(&name));
    if let Err(msg) = crate::validate::validate(schema.as_ref(), &body.values) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": msg})),
        )
            .into_response();
    }
    if let Some(bad_key) = body.values.keys().find(|k| k.starts_with('$')) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("`{bad_key}` is a reserved key")})),
        )
            .into_response();
    }
    let secret_keys: BTreeSet<&str> = schema
        .as_ref()
        .map(|s| {
            s.fields
                .iter()
                .filter(|f| f.is_secret())
                .map(|f| f.key.as_str())
                .collect()
        })
        .unwrap_or_default();

    for (key, value) in &body.values {
        if let Err(e) = state
            .store
            .skill_setting_set(&name, key, value, secret_keys.contains(key.as_str()))
            .await
        {
            return internal_error(e);
        }
    }

    // Recompute the allowlist from the FULL merged value set (old + new),
    // so saving one field doesn't drop hosts implied by others.
    if let Some(schema) = &schema {
        let rows = match state.store.skill_settings_for(&name).await {
            Ok(r) => r,
            Err(e) => return internal_error(e),
        };
        let base = state.base_per_skill.get(&name).cloned().unwrap_or_default();
        let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
        let merged = apply_settings(&base, &pairs);
        let hosts = crate::validate::derived_allowlist(schema, &merged.config);
        if !hosts.is_empty() {
            let json = serde_json::to_string(&hosts).expect("Vec<String> serializes");
            if let Err(e) = state
                .store
                .skill_setting_set(&name, HTTP_ALLOWLIST_KEY, &json, false)
                .await
            {
                return internal_error(e);
            }
        }
    }

    let reload_error = reload_skill(&state, &name).await.err();
    Json(serde_json::json!({"ok": true, "reload_error": reload_error})).into_response()
}

/// Rebuild the skill's merged config and reload its plugin in place.
/// `Ok(())` when the admin runs without a skill runtime (config still saved).
pub(crate) async fn reload_skill(state: &AppState, name: &str) -> Result<(), String> {
    let Some(handle) = &state.skills else {
        return Ok(());
    };
    let wasm = handle.dir.join(format!("{name}.wasm"));
    if !wasm.is_file() {
        return Ok(()); // not installed yet; config waits for the upload
    }
    let rows = state
        .store
        .skill_settings_for(name)
        .await
        .map_err(|e| e.to_string())?;
    let base = state.base_per_skill.get(name).cloned().unwrap_or_default();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let merged = apply_settings(&base, &pairs);

    let mut deps = handle.deps.clone();
    deps.per_skill.insert(name.to_string(), merged);
    // reload_path is synchronous plugin construction — run it off the
    // async worker thread.
    let registry = handle.registry.clone();
    let res = tokio::task::spawn_blocking(move || registry.reload_path(&wasm, &deps))
        .await
        .map_err(|e| e.to_string())?;
    res.map(|_| ()).map_err(|e| e.to_string())
}
