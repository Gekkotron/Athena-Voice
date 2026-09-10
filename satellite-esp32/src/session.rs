//! MQTT session state machine for the `athena/sat/<id>/session/<uuid>/…`
//! satellite protocol (see the repo README, "Satellite protocol").
//!
//! Hardware-free by design: inputs in, commands out. `main.rs` owns the
//! I2S/MQTT/GPIO handles and executes the commands; this module is unit
//! tested on the host (`./test.sh`).

/// Hard cap on how long one utterance may stream before the satellite
/// force-closes it (the runtime also has its own VAD).
pub const UTTERANCE_MAX_MS: u64 = 10_000;
/// How long to wait for the runtime's reply (transcript/tts/done) before
/// giving up and returning to idle — matches the runtime's own 30 s
/// status backstop.
pub const REPLY_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug)]
pub enum Input {
    /// Button press (later: wake word detection).
    Trigger,
    /// One captured mic frame, s16le mono 16 kHz bytes.
    MicFrame(Vec<u8>),
    /// End of utterance detected by the mic's silence tracker.
    SilenceDetected,
    /// Any MQTT message received on the subscribed filters.
    Inbound { topic: String, payload: Vec<u8> },
    /// Periodic timer, drives the timeout transitions.
    Tick,
}

#[derive(Debug)]
pub enum Command {
    Publish { topic: String, payload: Vec<u8> },
    ConfigureSpeaker { sample_rate: u32 },
    Play(Vec<u8>),
    /// What STT heard — shown on the serial monitor (a satellite
    /// without an amp is still fully usable that way).
    ShowTranscript { text: String, is_final: bool },
    /// The spoken answer as text, from `tts/text`.
    ShowAnswer(String),
    /// Back to idle — hook for LEDs/logging and re-arming the mic.
    SessionEnded,
}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Streaming,
    AwaitingReply,
    Playing,
}

pub struct Session {
    sat_id: String,
    locale: String,
    state: State,
    sid: Option<String>,
    deadline_ms: Option<u64>,
}

impl Session {
    pub fn new(sat_id: &str, locale: &str) -> Self {
        Self {
            sat_id: sat_id.to_string(),
            locale: locale.to_string(),
            state: State::Idle,
            sid: None,
            deadline_ms: None,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// The five response filters `main.rs` subscribes to once at connect.
    /// Never the `athena/sat/<id>/#` wildcard — the broker would echo our
    /// own audio stream back at us.
    pub fn subscriptions(sat_id: &str) -> [String; 5] {
        let base = format!("athena/sat/{sat_id}/session/+");
        [
            format!("{base}/transcript"),
            format!("{base}/tts"),
            format!("{base}/tts/meta"),
            format!("{base}/tts/text"),
            format!("{base}/done"),
        ]
    }

    pub fn handle(&mut self, input: Input, now_ms: u64) -> Vec<Command> {
        match input {
            Input::Trigger => self.on_trigger(now_ms),
            Input::MicFrame(bytes) => self.on_mic_frame(bytes),
            Input::SilenceDetected => self.end_utterance(now_ms),
            Input::Inbound { topic, payload } => self.on_inbound(&topic, payload, now_ms),
            Input::Tick => self.on_tick(now_ms),
        }
    }

    fn topic(&self, suffix: &str) -> String {
        let sid = self.sid.as_deref().unwrap_or_default();
        format!("athena/sat/{}/session/{sid}/{suffix}", self.sat_id)
    }

    fn on_trigger(&mut self, now_ms: u64) -> Vec<Command> {
        if self.state != State::Idle {
            return Vec::new();
        }
        self.sid = Some(uuid::Uuid::new_v4().to_string());
        self.state = State::Streaming;
        self.deadline_ms = Some(now_ms + UTTERANCE_MAX_MS);
        vec![Command::Publish {
            topic: self.topic("start"),
            payload: format!(r#"{{"locale":"{}"}}"#, self.locale).into_bytes(),
        }]
    }

    fn on_mic_frame(&mut self, bytes: Vec<u8>) -> Vec<Command> {
        if self.state != State::Streaming {
            return Vec::new();
        }
        vec![Command::Publish {
            topic: self.topic("audio"),
            payload: bytes,
        }]
    }

    /// Empty audio payload = end of utterance (protocol contract).
    fn end_utterance(&mut self, now_ms: u64) -> Vec<Command> {
        if self.state != State::Streaming {
            return Vec::new();
        }
        self.state = State::AwaitingReply;
        self.deadline_ms = Some(now_ms + REPLY_TIMEOUT_MS);
        vec![Command::Publish {
            topic: self.topic("audio"),
            payload: Vec::new(),
        }]
    }

    fn on_inbound(&mut self, topic: &str, payload: Vec<u8>, _now_ms: u64) -> Vec<Command> {
        if !matches!(self.state, State::AwaitingReply | State::Playing) {
            return Vec::new();
        }
        let Some(sid) = self.sid.as_deref() else {
            return Vec::new();
        };
        let prefix = format!("athena/sat/{}/session/{sid}/", self.sat_id);
        let Some(suffix) = topic.strip_prefix(&prefix) else {
            return Vec::new();
        };
        match suffix {
            "tts/meta" => {
                let sample_rate = serde_json::from_slice::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|v| v.get("sample_rate")?.as_u64())
                    .and_then(|r| u32::try_from(r).ok());
                self.state = State::Playing;
                match sample_rate {
                    Some(sample_rate) => vec![Command::ConfigureSpeaker { sample_rate }],
                    None => Vec::new(),
                }
            }
            "tts" => {
                self.state = State::Playing;
                vec![Command::Play(payload)]
            }
            "done" => self.finish(),
            "transcript" => {
                let Some(v) = json(&payload) else {
                    return Vec::new();
                };
                match v.get("text").and_then(serde_json::Value::as_str) {
                    Some(text) => vec![Command::ShowTranscript {
                        text: text.to_string(),
                        is_final: v
                            .get("is_final")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    }],
                    None => Vec::new(),
                }
            }
            "tts/text" => {
                match json(&payload)
                    .as_ref()
                    .and_then(|v| v.get("text")?.as_str())
                {
                    Some(text) => vec![Command::ShowAnswer(text.to_string())],
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn on_tick(&mut self, now_ms: u64) -> Vec<Command> {
        let expired = self.deadline_ms.is_some_and(|d| now_ms > d);
        if !expired {
            return Vec::new();
        }
        match self.state {
            State::Idle => Vec::new(),
            State::Streaming => self.end_utterance(now_ms),
            State::AwaitingReply | State::Playing => self.finish(),
        }
    }

    fn finish(&mut self) -> Vec<Command> {
        self.state = State::Idle;
        self.sid = None;
        self.deadline_ms = None;
        vec![Command::SessionEnded]
    }
}

fn json(payload: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid_from_start_topic(cmds: &[Command]) -> String {
        let Command::Publish { topic, .. } = &cmds[0] else {
            panic!("expected a publish command, got {cmds:?}");
        };
        topic.split('/').nth(4).unwrap().to_string()
    }

    fn started(now_ms: u64) -> (Session, String) {
        let mut s = Session::new("esp32-sat", "fr");
        let sid = sid_from_start_topic(&s.handle(Input::Trigger, now_ms));
        (s, sid)
    }

    #[test]
    fn trigger_starts_session_with_uuid_and_locale() {
        let mut s = Session::new("esp32-sat", "fr");
        let cmds = s.handle(Input::Trigger, 0);
        let Command::Publish { topic, payload } = &cmds[0] else {
            panic!("expected publish");
        };
        assert!(topic.starts_with("athena/sat/esp32-sat/session/"));
        assert!(topic.ends_with("/start"));
        let sid = topic.split('/').nth(4).unwrap();
        assert!(uuid::Uuid::parse_str(sid).is_ok());
        assert_eq!(payload, br#"{"locale":"fr"}"#);
        assert_eq!(*s.state(), State::Streaming);
    }

    #[test]
    fn mic_frames_are_forwarded_while_streaming() {
        let (mut s, sid) = started(0);
        let cmds = s.handle(Input::MicFrame(vec![1, 2, 3, 4]), 100);
        let Command::Publish { topic, payload } = &cmds[0] else {
            panic!("expected publish");
        };
        assert_eq!(topic, &format!("athena/sat/esp32-sat/session/{sid}/audio"));
        assert_eq!(payload, &[1, 2, 3, 4]);
    }

    #[test]
    fn silence_publishes_empty_audio_and_awaits_reply() {
        let (mut s, sid) = started(0);
        let cmds = s.handle(Input::SilenceDetected, 1000);
        let Command::Publish { topic, payload } = &cmds[0] else {
            panic!("expected publish");
        };
        assert_eq!(topic, &format!("athena/sat/esp32-sat/session/{sid}/audio"));
        assert!(payload.is_empty());
        assert_eq!(*s.state(), State::AwaitingReply);
    }

    #[test]
    fn utterance_deadline_forces_end_of_utterance() {
        let (mut s, sid) = started(0);
        let cmds = s.handle(Input::Tick, UTTERANCE_MAX_MS + 1);
        let Command::Publish { topic, payload } = &cmds[0] else {
            panic!("expected publish");
        };
        assert_eq!(topic, &format!("athena/sat/esp32-sat/session/{sid}/audio"));
        assert!(payload.is_empty());
        assert_eq!(*s.state(), State::AwaitingReply);
    }

    #[test]
    fn tts_meta_configures_speaker_and_moves_to_playing() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/tts/meta"),
                payload: br#"{"sample_rate":24000,"channels":1,"frame_ms":20,"codec":"opus"}"#
                    .to_vec(),
            },
            2000,
        );
        assert!(matches!(
            cmds[0],
            Command::ConfigureSpeaker { sample_rate: 24000 }
        ));
        assert_eq!(*s.state(), State::Playing);
    }

    #[test]
    fn tts_chunks_are_played_even_before_meta() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/tts"),
                payload: vec![9, 9],
            },
            2000,
        );
        let Command::Play(bytes) = &cmds[0] else {
            panic!("expected play");
        };
        assert_eq!(bytes, &[9, 9]);
    }

    #[test]
    fn transcript_is_surfaced_for_display() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/transcript"),
                payload: br#"{"text":"quelle heure est-il","is_final":true}"#.to_vec(),
            },
            2000,
        );
        let Command::ShowTranscript { text, is_final } = &cmds[0] else {
            panic!("expected ShowTranscript, got {cmds:?}");
        };
        assert_eq!(text, "quelle heure est-il");
        assert!(is_final);
    }

    #[test]
    fn answer_text_is_surfaced_for_display() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/tts/text"),
                payload: br#"{"text":"il est 15 h 14"}"#.to_vec(),
            },
            2000,
        );
        let Command::ShowAnswer(text) = &cmds[0] else {
            panic!("expected ShowAnswer, got {cmds:?}");
        };
        assert_eq!(text, "il est 15 h 14");
    }

    #[test]
    fn malformed_display_payloads_are_ignored() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/tts/text"),
                payload: b"not json".to_vec(),
            },
            2000,
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn done_ends_session() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/done"),
                payload: br#"{"outcome":"ok"}"#.to_vec(),
            },
            2000,
        );
        assert!(matches!(cmds[0], Command::SessionEnded));
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn reply_timeout_returns_to_idle() {
        let (mut s, _sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(Input::Tick, 1000 + REPLY_TIMEOUT_MS + 1);
        assert!(matches!(cmds[0], Command::SessionEnded));
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn playing_also_times_out() {
        let (mut s, sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        s.handle(
            Input::Inbound {
                topic: format!("athena/sat/esp32-sat/session/{sid}/tts/meta"),
                payload: br#"{"sample_rate":24000}"#.to_vec(),
            },
            2000,
        );
        let cmds = s.handle(Input::Tick, 1000 + REPLY_TIMEOUT_MS + 1);
        assert!(matches!(cmds[0], Command::SessionEnded));
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn inbound_for_other_session_is_ignored() {
        let (mut s, _sid) = started(0);
        s.handle(Input::SilenceDetected, 1000);
        let cmds = s.handle(
            Input::Inbound {
                topic: "athena/sat/esp32-sat/session/00000000-0000-4000-8000-000000000000/done"
                    .into(),
                payload: vec![],
            },
            2000,
        );
        assert!(cmds.is_empty());
        assert_eq!(*s.state(), State::AwaitingReply);
    }

    #[test]
    fn trigger_while_busy_is_ignored() {
        let (mut s, _sid) = started(0);
        let cmds = s.handle(Input::Trigger, 100);
        assert!(cmds.is_empty());
        assert_eq!(*s.state(), State::Streaming);
    }

    #[test]
    fn mic_frames_outside_streaming_are_dropped() {
        let mut s = Session::new("esp32-sat", "fr");
        assert!(s.handle(Input::MicFrame(vec![1]), 0).is_empty());
    }

    #[test]
    fn subscriptions_cover_the_five_response_topics() {
        let subs = Session::subscriptions("esp32-sat");
        assert_eq!(
            subs,
            [
                "athena/sat/esp32-sat/session/+/transcript".to_string(),
                "athena/sat/esp32-sat/session/+/tts".to_string(),
                "athena/sat/esp32-sat/session/+/tts/meta".to_string(),
                "athena/sat/esp32-sat/session/+/tts/text".to_string(),
                "athena/sat/esp32-sat/session/+/done".to_string(),
            ]
        );
    }
}
