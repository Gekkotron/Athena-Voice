#![deny(warnings)]
//! Athena-Voice skill SDK — shared surface between host and guest.
//!
//! Plan 4 delivers the type vocabulary and a stub HostCtx; the Extism guest ABI
//! is wired in a later task.

pub mod host;
pub mod response;
pub mod skill;

pub use response::{SkillError, SkillResponse};
pub use skill::{Intent, PatternRule, Skill, SlotKind, SlotSpec};
