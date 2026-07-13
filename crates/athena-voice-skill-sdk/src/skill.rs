use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::host::HostCtx;
use crate::response::{SkillError, SkillResponse};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    String,
    Number,
    /// Restricted vocabulary; enforced by the pattern matcher on the host side.
    OneOf(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotSpec {
    pub name: String,
    pub kind: SlotKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRule {
    pub intent: String,
    /// Phrases with `{slot_name}` placeholders. Example: `"météo à {city}"`.
    pub phrases: Vec<String>,
    #[serde(default)]
    pub slots: Vec<SlotSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub name: String,
    #[serde(default)]
    pub slots: BTreeMap<String, serde_json::Value>,
}

/// Trait skills implement on the guest side. On the host side, we invoke it via
/// Extism's ABI.
pub trait Skill {
    fn name(&self) -> &str;
    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule>;
    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_rule_serde_roundtrip() {
        let rule = PatternRule {
            intent: "weather.query".into(),
            phrases: vec![
                "météo à {city}".into(),
                "quel temps fait-il à {city}".into(),
            ],
            slots: vec![SlotSpec {
                name: "city".into(),
                kind: SlotKind::String,
            }],
        };
        let json = serde_json::to_string(&rule).unwrap();
        let back: PatternRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.intent, rule.intent);
        assert_eq!(back.phrases, rule.phrases);
        assert_eq!(back.slots, rule.slots);
    }

    #[test]
    fn intent_default_slots_is_empty() {
        let intent: Intent = serde_json::from_str(r#"{"name":"noop"}"#).unwrap();
        assert!(intent.slots.is_empty());
    }

    #[test]
    fn slot_kind_one_of_serde() {
        let k = SlotKind::OneOf(vec!["salon".into(), "cuisine".into()]);
        let json = serde_json::to_string(&k).unwrap();
        let back: SlotKind = serde_json::from_str(&json).unwrap();
        assert_eq!(k, back);
    }
}
