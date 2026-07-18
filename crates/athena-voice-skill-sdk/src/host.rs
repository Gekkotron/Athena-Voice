//! Guest-side view of the host functions.
//!
//! Plan 4 Task 3 defined `HostCtx` as a stub trait shape so host-side code
//! (Task 5) could compile against it. Task 8 replaces the stubs with real
//! `extism_pdk::host_fn` bindings on the guest side: every method now invokes
//! the corresponding host function declared in
//! `crates/athena-voice-runtime/src/wasm/host_fns.rs`. On non-wasm targets
//! (host workspace builds, host unit tests) the stubs remain so `HostCtx`
//! stays constructible without pulling in the guest ABI.

use crate::response::SkillError;

pub struct HostCtx {
    _priv: (),
}

impl HostCtx {
    /// Constructs a `HostCtx`. On the guest side this is what a skill's
    /// exported `handle` entry point receives; on the host side it is only
    /// useful in unit tests.
    #[must_use]
    pub fn for_testing() -> Self {
        Self { _priv: () }
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

    /// Reads a config value scoped to this skill.
    #[must_use]
    pub fn config_get(&self, _key: &str) -> Option<String> {
        None
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
        fn host_config_get(key: String) -> String;
        fn host_state_get(key: String) -> Vec<u8>;
        fn host_state_set(key: String, val: Vec<u8>);
        fn host_mqtt_publish(topic: String, payload: Vec<u8>) -> i64;
        fn host_http_get_json(url: String) -> Vec<u8>;
        fn host_schedule_mqtt(fires_at_ms: Vec<u8>, topic: String, payload: Vec<u8>) -> Vec<u8>;
    }

    pub(super) fn log(level: &str, msg: &str) {
        let _ = unsafe { host_log(level.to_string(), msg.to_string()) };
    }

    pub(super) fn config_get(key: &str) -> Option<String> {
        match unsafe { host_config_get(key.to_string()) } {
            Ok(v) if v.is_empty() => None,
            Ok(v) => Some(v),
            Err(_) => None,
        }
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

    #[must_use]
    pub fn config_get(&self, key: &str) -> Option<String> {
        guest::config_get(key)
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
}
