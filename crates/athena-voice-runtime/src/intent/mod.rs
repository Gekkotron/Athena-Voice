//! Pattern matcher for the IntentRouter.
//!
//! Skills declare `PatternRule`s (via `athena-voice-skill-sdk`). The matcher
//! aggregates rules from all loaded skills, and on each final transcript
//! computes fuzzy similarity to each phrase, extracting slot values from
//! `{slot_name}` placeholders. The best-scoring rule above `MATCH_THRESHOLD`
//! wins.

pub mod engine;
pub mod loader;
pub mod rule;

pub use engine::{IntentMatch, IntentMatcher, MATCH_THRESHOLD};
pub use loader::RuleIndex;
pub use rule::{HostPatternRule, HostSlotKind, HostSlotSpec};
