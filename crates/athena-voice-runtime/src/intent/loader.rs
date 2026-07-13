//! Aggregates `PatternRule`s from all loaded skills, keyed by locale.
//!
//! Plan 4 Task 7 delivers the type + a builder; Task 6 (SkillRegistry) will
//! populate it by calling into each loaded WASM skill's exported
//! `pattern_rules(locale)` function.

use std::collections::HashMap;

use super::rule::HostPatternRule;

/// (rule, skill_name) — the skill_name is retained so a matcher hit can be
/// routed to the right skill for dispatch.
type RuleEntry = (HostPatternRule, String);

#[derive(Default)]
pub struct RuleIndex {
    by_locale: HashMap<String, Vec<RuleEntry>>,
}

impl RuleIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, locale: String, rule: HostPatternRule, skill: String) {
        self.by_locale
            .entry(locale)
            .or_default()
            .push((rule, skill));
    }

    #[must_use]
    pub fn for_locale(&self, locale: &str) -> Option<&[RuleEntry]> {
        self.by_locale.get(locale).map(std::vec::Vec::as_slice)
    }

    #[must_use]
    pub fn locale_count(&self) -> usize {
        self.by_locale.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::rule::{HostPatternRule, HostSlotSpec};

    fn r(intent: &str) -> HostPatternRule {
        HostPatternRule {
            intent: intent.into(),
            phrases: vec!["stub".into()],
            slots: Vec::<HostSlotSpec>::new(),
        }
    }

    #[test]
    fn insert_and_query() {
        let mut idx = RuleIndex::new();
        idx.insert("fr".into(), r("a"), "s1".into());
        idx.insert("fr".into(), r("b"), "s2".into());
        idx.insert("en".into(), r("c"), "s3".into());

        assert_eq!(idx.for_locale("fr").unwrap().len(), 2);
        assert_eq!(idx.for_locale("en").unwrap().len(), 1);
        assert!(idx.for_locale("ja").is_none());
        assert_eq!(idx.locale_count(), 2);
    }
}
