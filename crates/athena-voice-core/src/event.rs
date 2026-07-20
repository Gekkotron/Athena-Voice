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
        #[serde(default)]
        reason: LlmFallbackReason,
        #[serde(default)]
        slots: Vec<String>,
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
    /// A new user utterance interrupted work still in flight for the previous
    /// one. Downstream stages (TTS, sink) use this to flush queued speech so
    /// the previous response stops playing.
    BargeIn {
        session: SessionId,
        reason: BargeInReason,
    },
    /// A skill dispatch resolved after its utterance was superseded by a newer
    /// final transcript. The response is dropped rather than forwarded to
    /// TTS/LLM.
    SkillCancelled {
        session: SessionId,
        skill: String,
    },
    /// A skill was (re)loaded from disk by the hot-reload watcher.
    SkillReloaded {
        name: String,
    },
    /// The hot-reload watcher failed to (re)build the plugin for a file; the
    /// previously-loaded plugin (if any) remains in effect.
    SkillReloadFailed {
        name: String,
        reason: String,
    },
    /// A previously-scheduled MQTT event fired: the scheduler task published
    /// it and removed it from the store.
    ScheduledFired {
        skill: String,
        id: i64,
    },
    /// Runtime-emitted "skill wants to speak" notification. The scheduler
    /// task uses this to inject skill-triggered TTS (e.g. timer expiration)
    /// into the router's TTS pipeline without going through the intent
    /// matcher.
    SkillNotify {
        session: SessionId,
        skill: String,
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BargeInReason {
    /// Barge-in triggered by a newer final transcript arriving while a prior
    /// utterance's dispatch or TTS was still in flight.
    NewFinalTranscript,
    /// Reserved: barge-in triggered by VAD detecting speech onset. Not wired
    /// yet — requires a real VAD upgrade.
    VadSpeechStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmFallbackReason {
    /// No skill matched the transcript (below confidence threshold).
    #[default]
    NoMatch,
    /// A skill matched, but one or more slots could not be extracted.
    MissingSlots,
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
