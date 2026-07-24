use athena_voice_core::ids::{SatelliteId, SessionId};

pub const ROOT: &str = "athena";

#[must_use]
pub fn sat_wildcard() -> String {
    format!("{ROOT}/sat/+/session/#")
}

#[must_use]
pub fn session_transcript(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/transcript")
}

#[must_use]
pub fn session_tts(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/tts")
}

#[must_use]
pub fn session_tts_meta(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/tts/meta")
}

/// The text being synthesized, published alongside the audio chunks so
/// satellites can display the answer.
#[must_use]
pub fn session_tts_text(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/tts/text")
}

#[must_use]
pub fn session_done(sat: &SatelliteId, sid: SessionId) -> String {
    format!("{ROOT}/sat/{sat}/session/{sid}/done")
}

#[must_use]
pub fn event_topic(kind: &str) -> String {
    format!("{ROOT}/events/{kind}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTopic {
    Start { sat: SatelliteId, sid: SessionId },
    Audio { sat: SatelliteId, sid: SessionId },
    /// Raw UTF-8 utterance injected as a final transcript, bypassing STT.
    /// Lets text-only satellites (and humans testing with `mosquitto_pub`)
    /// drive the intent pipeline without sending audio.
    Text { sat: SatelliteId, sid: SessionId },
    End { sat: SatelliteId, sid: SessionId },
}

#[must_use]
pub fn parse_satellite_topic(topic: &str) -> Option<ParsedTopic> {
    // athena/sat/<sat_id>/session/<sid>/{start|audio|text|end}
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 6 || parts[0] != ROOT || parts[1] != "sat" || parts[3] != "session" {
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
        assert_eq!(sat_wildcard(), "athena/sat/+/session/#");
    }

    #[test]
    fn text_topic_parses() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/text");
        assert_eq!(
            parse_satellite_topic(&topic),
            Some(ParsedTopic::Text { sat: sat(), sid })
        );
    }

    #[test]
    fn transcript_topic_layout() {
        let sid = SessionId::new_v4();
        let s = session_transcript(&sat(), sid);
        assert!(s.starts_with("athena/sat/phone-01/session/"));
        assert!(s.ends_with("/transcript"));
        assert!(s.contains(&sid.to_string()));
    }

    #[test]
    fn parse_start() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/start");
        let parsed = parse_satellite_topic(&topic).expect("parses");
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
            parse_satellite_topic(&topic),
            Some(ParsedTopic::Audio { .. })
        ));
    }

    #[test]
    fn parse_end() {
        let sid = SessionId::new_v4();
        let topic = format!("athena/sat/phone-01/session/{sid}/end");
        assert!(matches!(
            parse_satellite_topic(&topic),
            Some(ParsedTopic::End { .. })
        ));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(parse_satellite_topic("random/topic").is_none());
        assert!(parse_satellite_topic("athena/sat/phone-01/session").is_none());
        assert!(parse_satellite_topic("athena/sat/phone-01/session/not-a-uuid/audio").is_none());
    }
}
