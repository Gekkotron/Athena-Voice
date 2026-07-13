//! Guest-side view of the host functions.
//!
//! Plan 4 Task 3 delivers the trait shape; the real Extism guest bindings are
//! wired in Task 8. In the meantime the methods are stubs so host-side code
//! (Task 5) can be written against the same `HostCtx` type without a working
//! guest ABI.

use crate::response::SkillError;

pub struct HostCtx {
    _priv: (),
}

impl HostCtx {
    /// Constructs a `HostCtx` — only useful in unit tests. Real skill code
    /// gets a `HostCtx` from Extism's export ABI (Task 8).
    #[must_use]
    pub fn for_testing() -> Self {
        Self { _priv: () }
    }

    /// Emits a structured log line. Wired in Task 8.
    pub fn log(&self, _level: &str, _msg: &str) {
        // stub — real impl calls the host_log Extism host_fn.
    }

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
            "HostCtx::http_get_json not yet wired".into(),
        ))
    }
}
