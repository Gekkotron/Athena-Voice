//! Guest-side view of the host functions.
//!
//! Plan 4 Task 3 defined `HostCtx` as a stub trait shape so host-side code
//! (Task 5) could compile against it. Task 8 replaces the stubs with real
//! `extism_pdk::host_fn` bindings on the guest side: every method now invokes
//! the corresponding host function declared in
//! `crates/athena-voice-runtime/src/wasm/host_fns.rs`. On non-wasm targets
//! (host workspace builds, host unit tests) the stubs remain so `HostCtx`
//! stays constructible without pulling in the guest ABI.

use std::collections::HashMap;

use crate::response::SkillError;

/// Parsed view of the skill's INI config bytes returned by `host_config_get`.
///
/// Usage: `let config = ctx.config_get()?;`
/// Access sections via `config.section("foo")`.
pub struct IniSlice {
    pub(crate) data: Vec<u8>,
}

impl IniSlice {
    /// Wraps raw INI bytes (as served by `host_config_get`) for parsing.
    #[must_use]
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Returns the key/value map of `section`, if present.
    ///
    /// Section and key names are normalized to lowercase by the INI parser.
    #[must_use]
    pub fn section(&self, section: &str) -> Option<HashMap<String, Option<String>>> {
        let text = std::str::from_utf8(&self.data).ok()?;
        let map = ini::macro_safe_read(text).ok()?;
        map.get(&section.to_lowercase()).cloned()
    }
}

/// Local wall-clock time served by `host_local_time`.
///
/// The WASI guest's own clock is UTC-only; the host adds the user's
/// timezone offset so skills can speak local time.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct LocalTime {
    /// Milliseconds since the unix epoch (UTC).
    pub epoch_ms: i64,
    /// Local timezone offset from UTC in seconds (positive east of GMT).
    pub offset_sec: i32,
}

impl LocalTime {
    fn local_secs(&self) -> i64 {
        self.epoch_ms / 1000 + i64::from(self.offset_sec)
    }

    /// Local hour of day, 0–23.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn hour(&self) -> u8 {
        (self.local_secs().rem_euclid(86_400) / 3_600) as u8
    }

    /// Local minute of hour, 0–59.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn minute(&self) -> u8 {
        (self.local_secs().rem_euclid(3_600) / 60) as u8
    }
}

pub struct HostCtx {
    skill_name: String,
}

impl HostCtx {
    /// Constructs a `HostCtx` with no skill name. On the guest side this is
    /// what a skill's exported `handle` entry point receives; on the host
    /// side it is only useful in unit tests.
    #[must_use]
    pub fn for_testing() -> Self {
        Self {
            skill_name: String::new(),
        }
    }

    /// Constructs a `HostCtx` bound to `skill_name`, which scopes the
    /// `tmp_set` / `tmp_get` namespace on the host.
    #[must_use]
    pub fn for_guest(skill_name: impl Into<String>) -> Self {
        Self {
            skill_name: skill_name.into(),
        }
    }

    /// The skill name this context is bound to (empty for [`Self::for_testing`]).
    #[must_use]
    pub fn skill_name(&self) -> &str {
        &self.skill_name
    }
}

// ---------------------------------------------------------------------------
// Host-side stubs — kept so `HostCtx` remains constructible in host tests.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
impl HostCtx {
    /// Emits a structured log line. No-op on the host; the real call happens
    /// on the guest side via [`extism_pdk`].
    pub fn log(&self, _level: &str, _msg: &str) {}

    /// REACH: config_get always returns None on host.
    pub fn config_get(&self) -> Result<Option<IniSlice>, SkillError> {
        Ok(None)
    }

    /// Reads a config value scoped to this skill.
    #[must_use]
    pub fn config_get_toml(&self, _key: &str) -> Option<String> {
        None
    }

    /// REACH: local_time returns the unix epoch on the host.
    pub fn local_time(&self) -> Result<LocalTime, SkillError> {
        Ok(LocalTime {
            epoch_ms: 0,
            offset_sec: 0,
        })
    }

    /// REACH: tmp_set/tmp_get always return Ok(None) on host.
    pub fn tmp_set(&self, _key: &str, _val: &[u8], _expires_sec: u64) -> Result<(), SkillError> {
        Ok(())
    }

    pub fn tmp_get(&self, _key: &str) -> Result<Option<Vec<u8>>, SkillError> {
        Ok(None)
    }

    /// Reads a value from the per-skill KV store.
    pub fn state_get(&self, _key: &str) -> Result<Option<Vec<u8>>, SkillError> {
        Ok(None)
    }

    /// Writes a value to the per-skill KV store.
    pub fn state_set(&self, _key: &str, _val: &[u8]) -> Result<(), SkillError> {
        Ok(())
    }

    /// Publishes a message on MQTT under the skill's own namespace
    /// (`athena/skills/<skill>/`).
    pub fn mqtt_publish(&self, _topic: &str, _payload: &[u8]) -> Result<(), SkillError> {
        Ok(())
    }

    /// Fetches JSON from an allowlisted URL.
    pub fn http_get_json(&self, _url: &str) -> Result<serde_json::Value, SkillError> {
        Err(SkillError::HttpFailed(
            "HostCtx::http_get_json only wired on wasm guest".into(),
        ))
    }

    /// Schedules a future MQTT publish under the skill's own namespace,
    /// returning the row id of the scheduled event.
    pub fn schedule_mqtt(
        &self,
        _fires_at_ms: i64,
        _topic: &str,
        _payload: &[u8],
    ) -> Result<i64, SkillError> {
        Ok(0)
    }

    /// Plays raw PCM samples. No-op on the host.
    pub fn play_pcm(&self, _sample_rate: u32, _samples: &[f32]) -> Result<(), SkillError> {
        Ok(())
    }

    /// Plays Opus-encoded frames. No-op on the host.
    pub fn play_opus(&self, _frames: &[u8]) -> Result<(), SkillError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Guest-side bindings — real Extism host_fn imports.
// ---------------------------------------------------------------------------
#[cfg(target_family = "wasm")]
mod guest {
    use extism_pdk::host_fn;
    use serde_json::Value;

    use crate::response::SkillError;

    // Result codes mirror `MQTT_OK` / `MQTT_ERR_ACL` / `MQTT_ERR_CLIENT` in
    // `crates/athena-voice-runtime/src/wasm/host_fns.rs`.
    const MQTT_OK: i64 = 0;
    const MQTT_ERR_ACL: i64 = 1;
    const MQTT_ERR_CLIENT: i64 = 2;

    // Mirrors `SCHED_ERR_ACL` / `SCHED_ERR_STORE` in
    // `crates/athena-voice-runtime/src/wasm/host_fns.rs`. A non-negative
    // return is the scheduled event's row id.
    const SCHED_ERR_ACL: i64 = -1;
    const SCHED_ERR_STORE: i64 = -2;

    #[host_fn]
    unsafe extern "ExtismHost" {
        fn host_log(level: String, msg: String);
        fn host_config_get() -> Vec<u8>;
        fn host_local_time() -> Vec<u8>;
        fn host_state_get(key: String) -> Vec<u8>;
        fn host_state_set(key: String, val: Vec<u8>);
        fn host_mqtt_publish(topic: String, payload: Vec<u8>) -> i64;
        fn host_http_get_json(url: String) -> Vec<u8>;
        fn host_schedule_mqtt(fires_at_ms: Vec<u8>, topic: String, payload: Vec<u8>) -> Vec<u8>;
        fn host_play_pcm(sample_rate: u32, samples: Vec<u8>);
        fn host_play_opus(frames: Vec<u8>);
        fn host_tmp_set(skill: String, key: String, val: Vec<u8>, expires_sec: u64);
        fn host_tmp_get(skill: String, key: String) -> Vec<u8>;
    }

    pub(super) fn log(level: &str, msg: &str) {
        let _ = unsafe { host_log(level.to_string(), msg.to_string()) };
    }

    pub(super) fn config_get() -> Result<Vec<u8>, SkillError> {
        unsafe { host_config_get() }.map_err(|e| SkillError::Config(e.to_string()))
    }

    pub(super) fn local_time() -> Result<Vec<u8>, SkillError> {
        unsafe { host_local_time() }.map_err(|e| SkillError::HostFn(e.to_string()))
    }

    pub(super) fn state_get(key: &str) -> Result<Option<Vec<u8>>, SkillError> {
        let bytes = unsafe { host_state_get(key.to_string()) }
            .map_err(|e| SkillError::State(e.to_string()))?;
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }

    pub(super) fn state_set(key: &str, val: &[u8]) -> Result<(), SkillError> {
        unsafe { host_state_set(key.to_string(), val.to_vec()) }
            .map_err(|e| SkillError::State(e.to_string()))
    }

    pub(super) fn mqtt_publish(topic: &str, payload: &[u8]) -> Result<(), SkillError> {
        let code = unsafe { host_mqtt_publish(topic.to_string(), payload.to_vec()) }
            .map_err(|e| SkillError::MqttFailed(e.to_string()))?;
        match code {
            MQTT_OK => Ok(()),
            MQTT_ERR_ACL => Err(SkillError::MqttFailed(format!(
                "topic {topic} outside skill ACL namespace"
            ))),
            MQTT_ERR_CLIENT => Err(SkillError::MqttFailed("mqtt client error".into())),
            other => Err(SkillError::MqttFailed(format!(
                "unknown mqtt result code {other}"
            ))),
        }
    }

    pub(super) fn schedule_mqtt(
        fires_at_ms: i64,
        topic: &str,
        payload: &[u8],
    ) -> Result<i64, SkillError> {
        let bytes = unsafe {
            host_schedule_mqtt(
                fires_at_ms.to_le_bytes().to_vec(),
                topic.to_string(),
                payload.to_vec(),
            )
        }
        .map_err(|e| SkillError::MqttFailed(e.to_string()))?;
        let code = i64::from_le_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| SkillError::MqttFailed("malformed schedule_mqtt reply".into()))?,
        );
        match code {
            SCHED_ERR_ACL => Err(SkillError::MqttFailed(format!(
                "topic {topic} outside skill ACL namespace"
            ))),
            SCHED_ERR_STORE => Err(SkillError::MqttFailed("scheduler store error".into())),
            id if id >= 0 => Ok(id),
            other => Err(SkillError::MqttFailed(format!(
                "unknown schedule_mqtt result code {other}"
            ))),
        }
    }

    pub(super) fn play_pcm(sample_rate: u32, samples: Vec<u8>) -> Result<(), SkillError> {
        unsafe { host_play_pcm(sample_rate, samples) }
            .map_err(|e| SkillError::HostFn(e.to_string()))
    }

    pub(super) fn play_opus(frames: &[u8]) -> Result<(), SkillError> {
        unsafe { host_play_opus(frames.to_vec()) }.map_err(|e| SkillError::HostFn(e.to_string()))
    }

    pub(super) fn tmp_set(
        skill: &str,
        key: &str,
        val: &[u8],
        expires_sec: u64,
    ) -> Result<(), SkillError> {
        unsafe { host_tmp_set(skill.to_string(), key.to_string(), val.to_vec(), expires_sec) }
            .map_err(|e| SkillError::HostFn(e.to_string()))
    }

    pub(super) fn tmp_get(skill: &str, key: &str) -> Result<Vec<u8>, SkillError> {
        unsafe { host_tmp_get(skill.to_string(), key.to_string()) }
            .map_err(|e| SkillError::HostFn(e.to_string()))
    }

    pub(super) fn http_get_json(url: &str) -> Result<Value, SkillError> {
        let bytes = unsafe { host_http_get_json(url.to_string()) }
            .map_err(|e| SkillError::HttpFailed(e.to_string()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| SkillError::HttpFailed(format!("decode: {e}")))?;
        // Host encodes error envelopes as `{ "error": "..." }`; surface those
        // as `SkillError::HttpFailed` so skill code can `?` naturally.
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            return Err(SkillError::HttpFailed(err.to_string()));
        }
        Ok(value)
    }
}

#[cfg(target_family = "wasm")]
impl HostCtx {
    pub fn log(&self, level: &str, msg: &str) {
        guest::log(level, msg);
    }

    /// Gets the skill's config.
    ///
    /// Returns:
    /// - `Ok(Some(IniSlice))` when the host has config bytes for this skill
    ///   (`[skills] config_file` INI contents, or the TOML map as JSON)
    /// - `Ok(None)` when the host has no config for this skill
    pub fn config_get(&self) -> Result<Option<IniSlice>, SkillError> {
        let bytes = guest::config_get()?;
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(IniSlice { data: bytes }))
        }
    }

    /// Gets a TOML config value (backwards compatible). Only works when the
    /// host serves the legacy TOML map (encoded as a JSON object).
    #[must_use]
    pub fn config_get_toml(&self, key: &str) -> Option<String> {
        let bytes = guest::config_get().ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    /// The host's local wall-clock time (with timezone offset applied).
    pub fn local_time(&self) -> Result<LocalTime, SkillError> {
        let bytes = guest::local_time()?;
        serde_json::from_slice(&bytes)
            .map_err(|e| SkillError::HostFn(format!("local_time decode: {e}")))
    }

    pub fn state_get(&self, key: &str) -> Result<Option<Vec<u8>>, SkillError> {
        guest::state_get(key)
    }

    pub fn state_set(&self, key: &str, val: &[u8]) -> Result<(), SkillError> {
        guest::state_set(key, val)
    }

    pub fn mqtt_publish(&self, topic: &str, payload: &[u8]) -> Result<(), SkillError> {
        guest::mqtt_publish(topic, payload)
    }

    pub fn http_get_json(&self, url: &str) -> Result<serde_json::Value, SkillError> {
        guest::http_get_json(url)
    }

    pub fn schedule_mqtt(
        &self,
        fires_at_ms: i64,
        topic: &str,
        payload: &[u8],
    ) -> Result<i64, SkillError> {
        guest::schedule_mqtt(fires_at_ms, topic, payload)
    }

    pub fn play_pcm(&self, sample_rate: u32, samples: &[f32]) -> Result<(), SkillError> {
        let bytes = samples
            .iter()
            .flat_map(|&f| f.to_le_bytes())
            .collect::<Vec<_>>();
        guest::play_pcm(sample_rate, bytes)
    }

    pub fn play_opus(&self, frames: &[u8]) -> Result<(), SkillError> {
        guest::play_opus(frames)
    }

    /// Stores a short-lived value in the skill-scoped tmpfs namespace.
    pub fn tmp_set(&self, key: &str, val: &[u8], expires_sec: u64) -> Result<(), SkillError> {
        guest::tmp_set(self.skill_name(), key, val, expires_sec)
    }

    /// Reads a short-lived value; `Ok(None)` when missing or expired.
    pub fn tmp_get(&self, key: &str) -> Result<Option<Vec<u8>>, SkillError> {
        let bytes = guest::tmp_get(self.skill_name(), key)?;
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LocalTime;

    #[test]
    fn local_time_hour_minute_respect_offset() {
        // 1970-01-01 00:00 UTC at +02:00 → 02:00 local.
        let t = LocalTime {
            epoch_ms: 0,
            offset_sec: 7_200,
        };
        assert_eq!(t.hour(), 2);
        assert_eq!(t.minute(), 0);

        // 23:59:59 UTC at -01:00 → 22:59 local.
        let t = LocalTime {
            epoch_ms: 86_399_000,
            offset_sec: -3_600,
        };
        assert_eq!(t.hour(), 22);
        assert_eq!(t.minute(), 59);
    }
}
