//! Host-side mirror of the skill-SDK `PatternRule`. Kept separate so the
//! runtime can evolve its representation independently of the guest ABI.

use serde::{Deserialize, Serialize};

use athena_voice_skill_sdk::{PatternRule, SlotKind, SlotSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSlotKind {
    String,
    Number,
    OneOf(Vec<String>),
}

impl From<SlotKind> for HostSlotKind {
    fn from(k: SlotKind) -> Self {
        match k {
            SlotKind::String => Self::String,
            SlotKind::Number => Self::Number,
            SlotKind::OneOf(v) => Self::OneOf(v),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSlotSpec {
    pub name: String,
    pub kind: HostSlotKind,
}

impl From<SlotSpec> for HostSlotSpec {
    fn from(s: SlotSpec) -> Self {
        Self {
            name: s.name,
            kind: s.kind.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPatternRule {
    pub intent: String,
    pub phrases: Vec<String>,
    pub slots: Vec<HostSlotSpec>,
}

impl From<PatternRule> for HostPatternRule {
    fn from(r: PatternRule) -> Self {
        Self {
            intent: r.intent,
            phrases: r.phrases,
            slots: r.slots.into_iter().map(Into::into).collect(),
        }
    }
}
