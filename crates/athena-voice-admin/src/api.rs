//! JSON handlers for the admin API.

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use athena_voice_runtime::wasm::settings::{HTTP_ALLOWLIST_KEY, apply_settings};
use athena_voice_skill_sdk::ConfigSchema;

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

/// Union of: wasm files on disk, loaded registry names, names in the DB.
async fn known_skill_names(state: &AppState) -> anyhow::Result<BTreeSet<String>> {
    let mut names: BTreeSet<String> = state.base_per_skill.keys().cloned().collect();
    if let Some(handle) = &state.skills {
        names.extend(handle.registry.skill_names());
        if let Ok(entries) = std::fs::read_dir(&handle.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    names.insert(stem.to_string());
                }
            }
        }
    }
    for row in state.store.skill_settings_all().await? {
        names.insert(row.skill);
    }
    for name in state.store.skills_disabled().await? {
        names.insert(name);
    }
    Ok(names)
}

pub(crate) async fn skill_info(state: &AppState, name: &str) -> anyhow::Result<SkillInfo> {
    let rows = state.store.skill_settings_for(name).await?;
    let secret_keys: BTreeSet<String> = rows
        .iter()
        .filter(|r| r.is_secret)
        .map(|r| r.key.clone())
        .collect();
    let pairs: Vec<(String, String)> = rows.into_iter().map(|r| (r.key, r.value)).collect();
    let base = state.base_per_skill.get(name).cloned().unwrap_or_default();
    let merged = apply_settings(&base, &pairs);

    let (loaded, schema) = match &state.skills {
        Some(h) => (
            h.registry.skill_names().contains(&name.to_string()),
            h.registry.config_schema(name),
        ),
        None => (false, None),
    };
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

    let disabled = state.store.skills_disabled().await?;
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

    Ok(SkillInfo {
        name: name.to_string(),
        loaded,
        enabled: !disabled.contains(&name.to_string()),
        schema,
        config,
        http_allowlist: merged.http_allowlist,
    })
}

pub(crate) async fn list_skills(State(state): State<AppState>) -> Response {
    let names = match known_skill_names(&state).await {
        Ok(n) => n,
        Err(e) => return internal_error(e),
    };
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        match skill_info(&state, &name).await {
            Ok(info) => out.push(info),
            Err(e) => return internal_error(e),
        }
    }
    Json(out).into_response()
}
