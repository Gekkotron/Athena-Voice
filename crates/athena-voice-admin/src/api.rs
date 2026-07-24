//! JSON handlers for the admin API.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::Json;
use axum::extract::{Multipart, Path, State};
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
    Path(name): Path<String>,
    Json(body): Json<ConfigWrite>,
) -> Response {
    if !valid_skill_name(&name) {
        return invalid_skill_name_response();
    }
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

    // Fix 3: a blank value must never overwrite a stored secret — an empty
    // submission from the UI's masked secret field means "leave unchanged",
    // not "clear it". This is fetched once, up front, and covers BOTH the
    // schema-driven case (key marked secret in `secret_keys` above) and the
    // schema-less path (key already stored with `is_secret = true`, e.g. a
    // skill that isn't loaded yet so no schema is available).
    let existing_rows = match state.store.skill_settings_for(&name).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let existing_secret_keys: BTreeSet<String> = existing_rows
        .iter()
        .filter(|r| r.is_secret)
        .map(|r| r.key.clone())
        .collect();

    for (key, value) in &body.values {
        let is_secret =
            secret_keys.contains(key.as_str()) || existing_secret_keys.contains(key.as_str());
        if is_secret && value.trim().is_empty() {
            continue; // blank secret submission means "unchanged", not "clear"
        }
        if let Err(e) = state
            .store
            .skill_setting_set(&name, key, value, is_secret)
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
    // Fix 2: a disabled skill must stay unloaded. Without this check, a
    // config PUT for a skill whose `.wasm` file is still on disk (installed,
    // then disabled) would reload it straight back into the registry,
    // silently undoing the disable. Config is still saved either way — only
    // the reload is skipped.
    let disabled = state
        .store
        .skills_disabled()
        .await
        .map_err(|e| e.to_string())?;
    if disabled.iter().any(|d| d == name) {
        return Ok(());
    }
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

pub(crate) async fn enable_skill(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if !valid_skill_name(&name) {
        return invalid_skill_name_response();
    }
    if let Err(e) = state.store.skill_enabled_set(&name, true).await {
        return internal_error(e);
    }
    let reload_error = reload_skill(&state, &name).await.err();
    Json(serde_json::json!({"ok": true, "reload_error": reload_error})).into_response()
}

pub(crate) async fn disable_skill(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if !valid_skill_name(&name) {
        return invalid_skill_name_response();
    }
    if let Err(e) = state.store.skill_enabled_set(&name, false).await {
        return internal_error(e);
    }
    if let Some(handle) = &state.skills {
        handle.registry.remove(&name);
    }
    Json(serde_json::json!({"ok": true, "reload_error": null})).into_response()
}

/// `[a-z0-9_-]+` — the only path-safety guard between a user-supplied skill
/// name and the filesystem. `PathBuf::join` replaces the base entirely when
/// given an absolute path, so every handler that takes a `name` from the
/// request (path segment or multipart filename) must check this BEFORE that
/// name is used in a store call or a `dir.join(...)`.
fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn invalid_skill_name_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "skill name must match [a-z0-9_-]+"})),
    )
        .into_response()
}

pub(crate) async fn upload_skill(State(state): State<AppState>, mut parts: Multipart) -> Response {
    while let Ok(Some(field)) = parts.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let file_name = field.file_name().unwrap_or_default().to_string();
        let Some(name) = file_name.strip_suffix(".wasm") else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "file name must end in .wasm"})),
            )
                .into_response();
        };
        // Validate the name BEFORE checking whether a skills directory is
        // even configured: a malformed/traversal name is a client error
        // (400) regardless of server config, and must never reach the
        // filesystem check below either way.
        if !valid_skill_name(name) {
            return invalid_skill_name_response();
        }
        let Some(handle) = state.skills.clone() else {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "no skills directory configured"})),
            )
                .into_response();
        };
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return internal_error(e),
        };
        let dest = handle.dir.join(format!("{name}.wasm"));
        // Captured BEFORE writing: whether this name had a working skill
        // installed already. This decides the quarantine rule below.
        let existed_before = dest.is_file();
        if let Err(e) = tokio::fs::write(&dest, &bytes).await {
            return internal_error(e);
        }
        // Mark enabled (an upload is an explicit install) and load it.
        if let Err(e) = state.store.skill_enabled_set(name, true).await {
            return internal_error(e);
        }
        let reload_error = reload_skill(&state, name).await.err();
        // Fix 1: a failed reload on a brand-new file must not leave a
        // busted `.wasm` behind — the next process restart calls `load_dir`
        // over this same directory, and an un-quarantined bad file would
        // brick that skill's slot again (now Fix 1b makes `load_dir` tolerant
        // of ONE bad file, but there's no reason to keep a file we already
        // know is broken). If the file existed before (this was an overwrite
        // of a previously-working skill), we deliberately keep the new file
        // as-is: the running process still has the OLD plugin loaded (
        // `reload_path` leaves it untouched on failure), so nothing regresses
        // until a future restart — that tradeoff is unchanged by this fix.
        let removed = if reload_error.is_some() && !existed_before {
            let _ = tokio::fs::remove_file(&dest).await;
            true
        } else {
            false
        };
        return Json(
            serde_json::json!({"ok": true, "name": name, "reload_error": reload_error, "removed": removed}),
        )
        .into_response();
    }
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": "missing multipart field `file`"})),
    )
        .into_response()
}

pub(crate) async fn list_bundled(State(state): State<AppState>) -> Response {
    let mut out = Vec::new();
    if let Some(dir) = &state.bundled_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                out.push(serde_json::json!({"name": stem}));
            }
        }
    }
    Json(out).into_response()
}

pub(crate) async fn install_bundled(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    if !valid_skill_name(&name) {
        return invalid_skill_name_response();
    }
    let (Some(bundled), Some(handle)) = (state.bundled_dir.clone(), state.skills.clone()) else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "bundled skills not configured"})),
        )
            .into_response();
    };
    let src = bundled.join(format!("{name}.wasm"));
    if !src.is_file() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "unknown bundled skill"})),
        )
            .into_response();
    }
    let dest = handle.dir.join(format!("{name}.wasm"));
    // Captured BEFORE copying — same quarantine rule as `upload_skill`.
    let existed_before = dest.is_file();
    if let Err(e) = tokio::fs::copy(&src, &dest).await {
        return internal_error(e);
    }
    if let Err(e) = state.store.skill_enabled_set(&name, true).await {
        return internal_error(e);
    }
    let reload_error = reload_skill(&state, &name).await.err();
    // Fix 1: see the matching comment in `upload_skill` — a failed reload of
    // a never-before-installed skill must not leave a broken file for the
    // next restart to trip over; an overwrite of a previously-working skill
    // keeps the new file as-is.
    let removed = if reload_error.is_some() && !existed_before {
        let _ = tokio::fs::remove_file(&dest).await;
        true
    } else {
        false
    };
    Json(serde_json::json!({"ok": true, "reload_error": reload_error, "removed": removed}))
        .into_response()
}
