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

// Re-exported so guest skills (which live outside the workspace and pull the
// SDK in as a path dep) can perform fuzzy matching without adding their own
// direct `strsim` dependency.
pub use strsim;
