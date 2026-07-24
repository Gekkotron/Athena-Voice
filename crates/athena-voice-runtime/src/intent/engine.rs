//! Fuzzy pattern-matching engine with slot extraction.
//!
//! Given a candidate phrase like `"météo à {city}"` and an input like
//! `"quelle est la météo à Paris"`, the matcher:
//! 1. Splits the phrase into literal segments around `{slot}` placeholders.
//! 2. Locates each literal segment in the input.
//! 3. Extracts the text between segments as the slot value.
//! 4. Also computes a fuzzy similarity between the whole phrase (with slots
//!    filled) and the input, as a confidence signal.
//!
//! The best-scoring rule above `MATCH_THRESHOLD` wins.

use std::collections::BTreeMap;

use serde_json::json;
use strsim::normalized_damerau_levenshtein;

use athena_voice_skill_sdk::Intent;

use super::loader::RuleIndex;
use super::rule::{HostPatternRule, HostSlotKind};

pub const MATCH_THRESHOLD: f32 = 0.8;

/// The outcome of a successful match: the intent to dispatch + the skill that owns it.
#[derive(Debug, Clone)]
pub struct IntentMatch {
    pub intent: Intent,
    pub skill: String,
    pub confidence: f32,
}

pub struct IntentMatcher {
    // Wrapped so callers can Arc-share it.
}

impl IntentMatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }

    /// Runs the matcher against `text` for `locale`, returning the best match
    /// above `MATCH_THRESHOLD`, if any.
    #[must_use]
    pub fn find_match(&self, text: &str, locale: &str, index: &RuleIndex) -> Option<IntentMatch> {
        let rules = index.for_locale(locale)?;
        let mut best: Option<IntentMatch> = None;
        for (rule, skill) in rules {
            for phrase in &rule.phrases {
                if let Some(candidate) = try_match_phrase(phrase, rule, text)
                    && candidate.confidence >= MATCH_THRESHOLD
                {
                    let with_skill = IntentMatch {
                        intent: candidate.intent,
                        skill: skill.clone(),
                        confidence: candidate.confidence,
                    };
                    if best
                        .as_ref()
                        .is_none_or(|b| with_skill.confidence > b.confidence)
                    {
                        best = Some(with_skill);
                    }
                }
            }
        }
        best
    }
}

impl Default for IntentMatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn try_match_phrase(phrase: &str, rule: &HostPatternRule, input: &str) -> Option<IntentMatch> {
    let segments = split_phrase(phrase);
    let normalised_input = input.to_lowercase();

    // Slot-less phrases are matched purely fuzzily: requiring the literal as
    // an exact substring would make STT variations ("quel heure est-il" for
    // "quelle heure est-il") unmatchable even at 90% similarity — the
    // confidence threshold in `find_match` is the real gate. Real speech is
    // also padded ("dis-moi quelle heure est-il", hallucinated trailing
    // fragments), so the best contiguous word-window is scored too, at a
    // slight discount so exact whole-utterance matches win ties.
    if let [Segment::Literal(lit)] = segments.as_slice() {
        let lit_lower = lit.to_lowercase();
        let whole = normalized_damerau_levenshtein(&lit_lower, &normalised_input);
        let sim = whole.max(best_window_similarity(&lit_lower, &normalised_input) * 0.95);
        #[allow(clippy::cast_possible_truncation)]
        return Some(IntentMatch {
            intent: Intent {
                name: rule.intent.clone(),
                slots: BTreeMap::new(),
                locale: String::new(),
            },
            skill: String::new(),
            confidence: sim as f32,
        });
    }

    // Walk the segments, extracting slot values from the gaps.
    let mut cursor = 0usize;
    let mut slots: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut filled_phrase = String::new();
    for (i, seg) in segments.iter().enumerate() {
        match seg {
            Segment::Literal(lit) => {
                let lit_lower = lit.to_lowercase();
                match normalised_input[cursor..].find(&lit_lower) {
                    Some(hit) => {
                        cursor += hit + lit_lower.len();
                        filled_phrase.push_str(lit);
                    }
                    None => {
                        // A literal with trailing whitespace (right before a
                        // slot) can sit at the very end of the input when the
                        // slot value is missing — match it trimmed so the
                        // router can elicit the slot via LLM fallback.
                        let trimmed = lit_lower.trim_end();
                        let hit = normalised_input[cursor..].find(trimmed)?;
                        cursor += hit + trimmed.len();
                        filled_phrase.push_str(lit.trim_end());
                    }
                }
            }
            Segment::Slot(name) => {
                // Slot value spans from the cursor to the start of the next
                // literal (or end of input if this is the last segment).
                let end = match segments.get(i + 1) {
                    Some(Segment::Literal(next)) => {
                        let next_lower = next.to_lowercase();
                        cursor + normalised_input[cursor..].find(&next_lower).unwrap_or(0)
                    }
                    _ => normalised_input.len(),
                };
                let raw = input.get(cursor..end).map(str::trim);
                if let Some(raw) = raw {
                    if !raw.is_empty() && slot_matches_kind(raw, rule, name) {
                        slots.insert(name.clone(), json!(raw));
                        filled_phrase.push_str(raw);
                        cursor = end;
                    } else {
                        slots.insert(name.clone(), serde_json::Value::Null);
                    }
                } else {
                    slots.insert(name.clone(), serde_json::Value::Null);
                }
            }
        }
    }

    // Confidence: fuzzy similarity between the filled phrase and the actual
    // input, scaled by how much of the input we consumed.
    let sim = normalized_damerau_levenshtein(&filled_phrase.to_lowercase(), &normalised_input);
    #[allow(clippy::cast_possible_truncation)]
    let confidence = sim as f32;

    Some(IntentMatch {
        intent: Intent {
            name: rule.intent.clone(),
            slots,
            locale: String::new(),
        },
        skill: String::new(), // populated by IntentMatcher::find_match
        confidence,
    })
}

/// Highest similarity between `phrase` and any contiguous word window of
/// `input` sized within ±1 word of the phrase. Only meaningful when the
/// input is longer than the phrase (the whole-string comparison covers the
/// rest).
fn best_window_similarity(phrase: &str, input: &str) -> f64 {
    let words: Vec<&str> = input.split_whitespace().collect();
    let n = phrase.split_whitespace().count();
    if n == 0 || words.len() <= n {
        return 0.0;
    }
    let mut best = 0.0f64;
    for size in [n.saturating_sub(1).max(1), n, n + 1] {
        if size > words.len() {
            continue;
        }
        for start in 0..=(words.len() - size) {
            let window = words[start..start + size].join(" ");
            best = best.max(normalized_damerau_levenshtein(phrase, &window));
        }
    }
    best
}

fn slot_matches_kind(raw: &str, rule: &HostPatternRule, slot_name: &str) -> bool {
    let Some(spec) = rule.slots.iter().find(|s| s.name == slot_name) else {
        return true; // no spec = any value accepted
    };
    match &spec.kind {
        HostSlotKind::String => true,
        HostSlotKind::Number => raw.parse::<f64>().is_ok(),
        HostSlotKind::OneOf(options) => options.iter().any(|opt| opt.eq_ignore_ascii_case(raw)),
    }
}

#[derive(Debug)]
enum Segment {
    Literal(String),
    Slot(String),
}

fn split_phrase(phrase: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = phrase.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if !buf.is_empty() {
                out.push(Segment::Literal(std::mem::take(&mut buf)));
            }
            let mut name = String::new();
            for nc in chars.by_ref() {
                if nc == '}' {
                    break;
                }
                name.push(nc);
            }
            out.push(Segment::Slot(name));
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        out.push(Segment::Literal(buf));
    }
    out
}

#[cfg(test)]
mod tests {
    use athena_voice_skill_sdk::SlotKind;

    use super::*;
    use crate::intent::rule::{HostPatternRule, HostSlotKind, HostSlotSpec};

    fn rule(intent: &str, phrases: &[&str], slots: &[(&str, HostSlotKind)]) -> HostPatternRule {
        HostPatternRule {
            intent: intent.into(),
            phrases: phrases.iter().map(|&s| s.into()).collect(),
            slots: slots
                .iter()
                .map(|(n, k)| HostSlotSpec {
                    name: (*n).into(),
                    kind: k.clone(),
                })
                .collect(),
        }
    }

    fn index_with(rules: Vec<(HostPatternRule, &str, &str)>) -> RuleIndex {
        let mut idx = RuleIndex::default();
        for (r, locale, skill) in rules {
            idx.insert(locale.into(), r, skill.into());
        }
        idx
    }

    #[test]
    fn padded_utterance_matches_embedded_phrase() {
        // Real mic transcript: whisper appended a hallucinated fragment.
        let idx = index_with(vec![(
            rule("time.query", &["quelle heure est-il"], &[]),
            "fr",
            "clock",
        )]);
        let m = IntentMatcher::new()
            .find_match("Quelle heure est-il ? La nuit", "fr", &idx)
            .expect("padded utterance must still match");
        assert_eq!(m.intent.name, "time.query");

        // Polite framing around the phrase.
        let m = IntentMatcher::new()
            .find_match("dis-moi quelle heure est-il s'il te plaît", "fr", &idx)
            .expect("polite padding must still match");
        assert_eq!(m.intent.name, "time.query");

        // Unrelated long text must NOT match.
        assert!(
            IntentMatcher::new()
                .find_match("la nuit tous les chats sont gris vraiment", "fr", &idx)
                .is_none()
        );
    }

    #[test]
    fn stt_variation_of_slotless_phrase_still_matches() {
        // Real transcript from whisper hearing a human say "quelle heure
        // est-il" — close enough that the fuzzy threshold must accept it.
        let idx = index_with(vec![(
            rule("time.query", &["quelle heure est-il"], &[]),
            "fr",
            "clock",
        )]);
        let m = IntentMatcher::new()
            .find_match("Quel heure est-il", "fr", &idx)
            .expect("near-exact STT transcript must match");
        assert_eq!(m.intent.name, "time.query");
        assert!(m.confidence >= 0.85, "confidence {}", m.confidence);
    }

    #[test]
    fn exact_match_no_slots() {
        let idx = index_with(vec![(
            rule("time.query", &["quelle heure est-il"], &[]),
            "fr",
            "clock",
        )]);
        let m = IntentMatcher::new()
            .find_match("quelle heure est-il", "fr", &idx)
            .unwrap();
        assert_eq!(m.intent.name, "time.query");
        assert_eq!(m.skill, "clock");
        assert!(m.confidence >= 0.99);
    }

    #[test]
    fn slot_extraction_string() {
        let idx = index_with(vec![(
            rule(
                "weather.query",
                &["météo à {city}"],
                &[("city", HostSlotKind::String)],
            ),
            "fr",
            "weather",
        )]);
        let m = IntentMatcher::new()
            .find_match("météo à Paris", "fr", &idx)
            .unwrap();
        assert_eq!(m.intent.name, "weather.query");
        assert_eq!(m.intent.slots["city"], serde_json::json!("Paris"));
    }

    #[test]
    fn fuzzy_typo_below_threshold_rejected() {
        let idx = index_with(vec![(
            rule("time.query", &["quelle heure est-il"], &[]),
            "fr",
            "clock",
        )]);
        let m = IntentMatcher::new().find_match("bonjour ça va", "fr", &idx);
        assert!(m.is_none(), "unrelated text must not match");
    }

    #[test]
    fn number_slot_rejects_non_numeric() {
        let idx = index_with(vec![(
            rule(
                "timer.set",
                &["chronomètre de {minutes} minutes"],
                &[("minutes", HostSlotKind::Number)],
            ),
            "fr",
            "timer",
        )]);
        // Numeric input: should match.
        assert!(
            IntentMatcher::new()
                .find_match("chronomètre de 5 minutes", "fr", &idx)
                .is_some()
        );
        // Non-numeric: rejected.
        assert!(
            IntentMatcher::new()
                .find_match("chronomètre de longtemps minutes", "fr", &idx)
                .is_none()
        );
    }

    #[test]
    fn one_of_slot_restricts_vocabulary() {
        let idx = index_with(vec![(
            rule(
                "light.turn_on",
                &["allume la lumière du {room}"],
                &[(
                    "room",
                    HostSlotKind::OneOf(vec!["salon".into(), "cuisine".into()]),
                )],
            ),
            "fr",
            "lights",
        )]);
        assert!(
            IntentMatcher::new()
                .find_match("allume la lumière du salon", "fr", &idx)
                .is_some()
        );
        assert!(
            IntentMatcher::new()
                .find_match("allume la lumière du garage", "fr", &idx)
                .is_none(),
            "unknown room must be rejected by OneOf"
        );
    }

    #[test]
    fn best_of_multiple_rules_wins() {
        let idx = index_with(vec![
            (rule("greet.short", &["bonjour"], &[]), "fr", "greet"),
            (
                rule("greet.long", &["bonjour, comment allez-vous"], &[]),
                "fr",
                "greet",
            ),
        ]);
        let m = IntentMatcher::new()
            .find_match("bonjour, comment allez-vous", "fr", &idx)
            .unwrap();
        assert_eq!(m.intent.name, "greet.long");
    }

    #[test]
    fn wrong_locale_no_match() {
        let idx = index_with(vec![(
            rule("time.query", &["quelle heure est-il"], &[]),
            "fr",
            "clock",
        )]);
        // Same phrase, different locale key — no rules for "en" registered.
        assert!(
            IntentMatcher::new()
                .find_match("quelle heure est-il", "en", &idx)
                .is_none()
        );
    }

    #[test]
    fn slot_kind_from_sdk_roundtrip() {
        let sdk = SlotKind::Number;
        let host: HostSlotKind = sdk.into();
        assert!(matches!(host, HostSlotKind::Number));
    }
}
