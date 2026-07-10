use std::collections::BTreeMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::ids::SessionId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    pub session: SessionId,
    pub seq: u32,
    #[serde(with = "bytes_serde")]
    pub pcm: Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub name: String,
    pub slots: BTreeMap<String, serde_json::Value>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub finish: FinishReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Error,
}

mod bytes_serde {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(b: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD
            .decode(s.as_bytes())
            .map(Bytes::from)
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::ids::SessionId;

    #[test]
    fn audio_frame_serde_roundtrip() {
        let a = AudioFrame {
            session: SessionId::new_v4(),
            seq: 42,
            pcm: Bytes::from_static(&[1, 2, 3, 4]),
        };
        let json = serde_json::to_string(&a).unwrap();
        let b: AudioFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(a.session, b.session);
        assert_eq!(a.seq, b.seq);
        assert_eq!(a.pcm.as_ref(), b.pcm.as_ref());
    }

    #[test]
    fn transcript_serde_roundtrip_final() {
        let a = Transcript { text: "hello".into(), is_final: true, confidence: Some(0.95) };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"is_final\":true"));
        let b: Transcript = serde_json::from_str(&json).unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.is_final, b.is_final);
        assert_eq!(a.confidence, b.confidence);
    }

    #[test]
    fn transcript_serde_omits_none_confidence() {
        let a = Transcript { text: "hi".into(), is_final: false, confidence: None };
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("confidence"), "unexpected key present in {json}");
    }

    #[test]
    fn intent_serde_roundtrip_with_slots() {
        let mut slots = std::collections::BTreeMap::new();
        slots.insert("city".into(), serde_json::json!("Paris"));
        slots.insert("day".into(), serde_json::json!(1));
        let a = Intent { name: "weather.query".into(), slots, confidence: 0.87 };
        let json = serde_json::to_string(&a).unwrap();
        let b: Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(a.name, b.name);
        assert_eq!(a.slots, b.slots);
        assert!((a.confidence - b.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn completion_serde_roundtrip() {
        let a = Completion { text: "il fait beau".into(), finish: FinishReason::Stop };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"finish\":\"stop\""));
        let b: Completion = serde_json::from_str(&json).unwrap();
        assert_eq!(a.text, b.text);
        assert!(matches!(b.finish, FinishReason::Stop));
    }
}
