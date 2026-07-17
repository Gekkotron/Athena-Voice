//! Extism WASM host for Athena-Voice skills.
//!
//! Structure (populated across Plan 4 Tasks 4–8):
//! - `host_fns`   — Extism host function implementations (log, config_get,
//!   state_get/set, mqtt_publish, http_get_json).
//! - `registry`   — `SkillRegistry`: loads `*.wasm` files, caches Extism `Plugin`s,
//!   populates the `RuleIndex` by calling each skill's exported `pattern_rules`.
//! - `dispatcher` — `SkillDispatcher` actor that receives `(session_id, intent)`
//!   and calls into a plugin via `spawn_blocking`.
//!
//! Task 4 registered the module tree; Tasks 5–7 fill in host functions, the
//! rule index, and the skill registry.

pub mod dispatcher;
pub mod error;
pub mod host_fns;
pub mod registry;
