//! Topic layout and payload shapes for the assist bridge. The wire is FIXED
//! by the DomoticApp side: `{prefix}/transcription/{device}` in,
//! `{prefix}/tts/{device}` + `{prefix}/llm/{device}/status` out, all JSON.

#[must_use]
pub fn transcription_wildcard(prefix: &str) -> String {
    format!("{prefix}/transcription/+")
}

/// Extracts the device id from `{prefix}/transcription/{device}`. Returns
/// `None` for foreign topics and for device ids that are empty or contain
/// MQTT-special characters (`/`, `+`, `#`) — those would let a hostile
/// publisher steer the answer topic.
#[must_use]
pub fn parse_transcription(prefix: &str, topic: &str) -> Option<String> {
    let rest = topic.strip_prefix(prefix)?.strip_prefix('/')?;
    let device = rest.strip_prefix("transcription/")?;
    if device.is_empty() || device.contains(['/', '+', '#']) {
        return None;
    }
    Some(device.to_string())
}

#[must_use]
pub fn tts_topic(prefix: &str, device: &str) -> String {
    format!("{prefix}/tts/{device}")
}

#[must_use]
pub fn status_topic(prefix: &str, device: &str) -> String {
    format!("{prefix}/llm/{device}/status")
}

/// Parses the app's `{"text": "..."}` payload; `None` unless `text` is a
/// non-empty string after trimming.
#[must_use]
pub fn parse_text_payload(payload: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let text = v.get("text")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_shape() {
        assert_eq!(transcription_wildcard("assist"), "assist/transcription/+");
    }

    #[test]
    fn parse_extracts_device() {
        assert_eq!(
            parse_transcription("assist", "assist/transcription/pixel-7"),
            Some("pixel-7".to_string())
        );
    }

    #[test]
    fn parse_rejects_foreign_topics() {
        assert_eq!(parse_transcription("assist", "assist/tts/pixel-7"), None);
        assert_eq!(
            parse_transcription("assist", "athena/sat/x/session/y/text"),
            None
        );
        assert_eq!(parse_transcription("assist", "assist/transcription"), None);
        // Extra levels would mean the device id contained '/': reject.
        assert_eq!(
            parse_transcription("assist", "assist/transcription/a/b"),
            None
        );
    }

    #[test]
    fn parse_rejects_hostile_device_ids() {
        // '+'/'#' in a concrete (non-filter) topic are legal MQTT but would
        // let a publisher steer our answer topic — reject.
        assert_eq!(
            parse_transcription("assist", "assist/transcription/+"),
            None
        );
        assert_eq!(
            parse_transcription("assist", "assist/transcription/#"),
            None
        );
        assert_eq!(parse_transcription("assist", "assist/transcription/"), None);
    }

    #[test]
    fn outbound_topics() {
        assert_eq!(tts_topic("assist", "pixel"), "assist/tts/pixel");
        assert_eq!(status_topic("assist", "pixel"), "assist/llm/pixel/status");
    }

    #[test]
    fn payload_requires_nonempty_text() {
        assert_eq!(
            parse_text_payload(br#"{"text": "quelle heure est-il"}"#),
            Some("quelle heure est-il".to_string())
        );
        assert_eq!(parse_text_payload(br#"{"text": "  "}"#), None);
        assert_eq!(parse_text_payload(br#"{"other": 1}"#), None);
        assert_eq!(parse_text_payload(b"not json"), None);
        // Trims surrounding whitespace.
        assert_eq!(
            parse_text_payload(br#"{"text": " bonjour "}"#),
            Some("bonjour".to_string())
        );
    }
}
