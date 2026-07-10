use serde::{Deserialize, Serialize};

use crate::ids::{Locale, SatelliteId, SessionId};
use crate::types::Intent;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    },
    AudioFrameDropped {
        session: SessionId,
        seq: u32,
    },
    TranscriptPartial {
        session: SessionId,
        text: String,
    },
    TranscriptFinal {
        session: SessionId,
        text: String,
    },
    IntentMatched {
        session: SessionId,
        intent: Intent,
    },
    SkillInvoked {
        session: SessionId,
        skill: String,
    },
    SkillPanicked {
        session: SessionId,
        skill: String,
        reason: String,
    },
    LlmFallback {
        session: SessionId,
    },
    TtsChunk {
        session: SessionId,
        seq: u32,
        bytes_len: usize,
    },
    SessionEnded {
        session: SessionId,
        outcome: Outcome,
    },
    ProviderError {
        session: SessionId,
        stage: Stage,
        error: String,
    },
    CircuitOpened {
        stage: Stage,
        provider: String,
    },
    CircuitClosed {
        stage: Stage,
        provider: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
    Cancelled,
    Overloaded,
    Orphaned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Ingest,
    Vad,
    Stt,
    Router,
    Skill,
    Llm,
    Tts,
    Sink,
    Storage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{Locale, SatelliteId, SessionId};

    #[test]
    fn event_session_started_tagged_serde() {
        let e = Event::SessionStarted {
            session: SessionId::new_v4(),
            satellite: SatelliteId::new("phone-01").unwrap(),
            locale: Locale::new("fr").unwrap(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"kind\":\"session_started\""));
        let round: Event = serde_json::from_str(&json).unwrap();
        assert!(matches!(round, Event::SessionStarted { .. }));
    }

    #[test]
    fn event_session_ended_carries_outcome() {
        let e = Event::SessionEnded {
            session: SessionId::new_v4(),
            outcome: Outcome::Ok,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"outcome\":\"ok\""));
    }

    #[test]
    fn event_provider_error_carries_stage() {
        let e = Event::ProviderError {
            session: SessionId::new_v4(),
            stage: Stage::Stt,
            error: "timeout".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"stage\":\"stt\""));
        assert!(json.contains("\"error\":\"timeout\""));
    }

    #[test]
    fn outcome_variants_snake_case() {
        assert_eq!(serde_json::to_string(&Outcome::Ok).unwrap(), "\"ok\"");
        assert_eq!(
            serde_json::to_string(&Outcome::Overloaded).unwrap(),
            "\"overloaded\""
        );
        assert_eq!(
            serde_json::to_string(&Outcome::Orphaned).unwrap(),
            "\"orphaned\""
        );
    }
}
