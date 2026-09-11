use athena_voice_core::ids::{SatelliteId, SessionId};

/// Default namespace for every topic this runtime owns. Overridable
/// (`[mqtt] topic_root`) because a broker shared with other software may
/// already use `athena/` — satellites and workers must then be told the
/// same root.
pub const DEFAULT_ROOT: &str = "athena";

#[must_use]
pub fn sat_wildcard(root: &str) -> String {
    format!("{root}/sat/+/session/#")
}

#[must_use]
pub fn session_transcript(root: &str, sat: &SatelliteId, sid: SessionId) -> String {
    format!("{root}/sat/{sat}/session/{sid}/transcript")
}

#[must_use]
pub fn session_tts(root: &str, sat: &SatelliteId, sid: SessionId) -> String {
    format!("{root}/sat/{sat}/session/{sid}/tts")
}

#[must_use]
pub fn session_tts_meta(root: &str, sat: &SatelliteId, sid: SessionId) -> String {
    format!("{root}/sat/{sat}/session/{sid}/tts/meta")
}

/// The text being synthesized, published alongside the audio chunks so
/// satellites can display the answer.
#[must_use]
pub fn session_tts_text(root: &str, sat: &SatelliteId, sid: SessionId) -> String {
    format!("{root}/sat/{sat}/session/{sid}/tts/text")
}

#[must_use]
pub fn session_done(root: &str, sat: &SatelliteId, sid: SessionId) -> String {
    format!("{root}/sat/{sat}/session/{sid}/done")
}

#[must_use]
pub fn event_topic(root: &str, kind: &str) -> String {
    format!("{root}/events/{kind}")
}

/// Provider request/reply topics (`<root>/providers/<kind>/<name>/…`).
/// The STT/TTS workers must be started with the same root.
#[must_use]
pub fn provider_request(root: &str, kind: &str, name: &str) -> String {
    format!("{root}/providers/{kind}/{name}/request")
}

#[must_use]
pub fn provider_response(root: &str, kind: &str, name: &str) -> String {
    format!("{root}/providers/{kind}/{name}/response")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTopic {
    Start {
        sat: SatelliteId,
        sid: SessionId,
    },
    Audio {
        sat: SatelliteId,
        sid: SessionId,
    },
    /// Raw UTF-8 utterance injected as a final transcript, bypassing STT.
    /// Lets text-only satellites (and humans testing with `mosquitto_pub`)
    /// drive the intent pipeline without sending audio.
    Text {
        sat: SatelliteId,
        sid: SessionId,
    },
    End {
        sat: SatelliteId,
        sid: SessionId,
    },
}

#[must_use]
pub fn parse_satellite_topic(root: &str, topic: &str) -> Option<ParsedTopic> {
    // <root>/sat/<sat_id>/session/<sid>/{start|audio|text|end}
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 6 || parts[0] != root || parts[1] != "sat" || parts[3] != "session" {
        return None;
    }
    let sat = SatelliteId::new(parts[2]).ok()?;
    let sid: SessionId = parts[4].parse().ok()?;
    match parts[5] {
        "start" => Some(ParsedTopic::Start { sat, sid }),
        "audio" => Some(ParsedTopic::Audio { sat, sid }),
        "text" => Some(ParsedTopic::Text { sat, sid }),
        "end" => Some(ParsedTopic::End { sat, sid }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sat() -> SatelliteId {
        SatelliteId::new("phone-01").unwrap()
    }

    #[test]
    fn wildcard_matches_spec() {
        assert_eq!(sat_wildcard(DEFAULT_ROOT), "athena/sat/+/session/#");
    }

    #[test]
    fn text_topic_parses() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/text");
        assert_eq!(
            parse_satellite_topic(DEFAULT_ROOT, &topic),
            Some(ParsedTopic::Text { sat: sat(), sid })
        );
    }

    #[test]
    fn transcript_topic_layout() {
        let sid = SessionId::new_v4();
        let s = session_transcript(DEFAULT_ROOT, &sat(), sid);
        assert!(s.starts_with("athena/sat/phone-01/session/"));
        assert!(s.ends_with("/transcript"));
        assert!(s.contains(&sid.to_string()));
    }

    #[test]
    fn parse_start() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/start");
        let parsed = parse_satellite_topic(DEFAULT_ROOT, &topic).expect("parses");
        match parsed {
            ParsedTopic::Start { sat, sid: got } => {
                assert_eq!(sat.as_str(), "phone-01");
                assert_eq!(got, sid);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parse_audio() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/audio");
        assert!(matches!(
            parse_satellite_topic(DEFAULT_ROOT, &topic),
            Some(ParsedTopic::Audio { .. })
        ));
    }

    #[test]
    fn parse_end() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/end");
        assert!(matches!(
            parse_satellite_topic(DEFAULT_ROOT, &topic),
            Some(ParsedTopic::End { .. })
        ));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(parse_satellite_topic(DEFAULT_ROOT, "random/topic").is_none());
        assert!(parse_satellite_topic(DEFAULT_ROOT, "athena/sat/phone-01/session").is_none());
        assert!(parse_satellite_topic(DEFAULT_ROOT, "athena/sat/phone-01/session/not-a-uuid/audio").is_none());
    }

    #[test]
    fn a_custom_root_moves_every_topic() {
        let sat = SatelliteId::new("kitchen").unwrap();
        let sid = SessionId::new_v4();
        for t in [
            sat_wildcard("assist"),
            session_transcript("assist", &sat, sid),
            session_tts("assist", &sat, sid),
            session_tts_meta("assist", &sat, sid),
            session_tts_text("assist", &sat, sid),
            session_done("assist", &sat, sid),
            event_topic("assist", "session_started"),
            provider_request("assist", "stt", "whisper"),
            provider_response("assist", "tts", "say"),
        ] {
            assert!(t.starts_with("assist/"), "{t} kept the old root");
            assert!(!t.contains("athena"), "{t} still mentions athena");
        }
    }

    #[test]
    fn parsing_is_root_scoped() {
        let sid = SessionId::new_v4();
        // A satellite publishing under a different root must be ignored,
        // so two deployments can share one broker.
        let topic = format!("assist/sat/phone-01/session/{sid}/audio");
        assert!(parse_satellite_topic("assist", &topic).is_some());
        assert!(parse_satellite_topic(DEFAULT_ROOT, &topic).is_none());
    }
}
