use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillResponse {
    /// Skill produced a spoken response.
    Speak { text: String },
    /// Skill executed silently (e.g. side-effect only).
    Empty,
    /// Skill wants the host to consult the LLM with the supplied prompt.
    AskLlm { prompt: String },
    /// Sampled PCM audio (f32) for immediate playback.
    SampledPcm {
        sample_rate: u32,
        samples: Vec<f32>,
    },
    /// Opus-encoded audio frames for efficient playback.
    SampledOpus {
        opus_frames: Vec<u8>,
    },
    /// Adjust the playback volume (0.0 = mute, 1.0 = nominal, 1.5 = 50% boost).
    Volume(f32),
}

impl SkillResponse {
    #[must_use]
    pub fn speak(text: impl Into<String>) -> Self {
        Self::Speak { text: text.into() }
    }
    #[must_use]
    pub fn empty() -> Self {
        Self::Empty
    }
    #[must_use]
    pub fn ask_llm(prompt: impl Into<String>) -> Self {
        Self::AskLlm {
            prompt: prompt.into(),
        }
    }
    #[must_use]
    pub fn sampled_pcm(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self::SampledPcm { sample_rate, samples }
    }
    #[must_use]
    pub fn sampled_opus(opus_frames: Vec<u8>) -> Self {
        Self::SampledOpus { opus_frames }
    }
    #[must_use]
    pub fn volume(level: f32) -> Self {
        Self::Volume(level.clamp(0.0, 1.5))
    }
}

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum SkillError {
    #[error("http failed: {0}")]
    HttpFailed(String),
    #[error("mqtt failed: {0}")]
    MqttFailed(String),
    #[error("state error: {0}")]
    State(String),
    #[error("custom: {0}")]
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speak_variant_serde() {
        let r = SkillResponse::speak("hello");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"kind\":\"speak\""));
        assert!(json.contains("\"text\":\"hello\""));
        let back: SkillResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, SkillResponse::Speak { text } if text == "hello"));
    }

    #[test]
    fn ask_llm_variant_serde() {
        let r = SkillResponse::ask_llm("what time is it");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"kind\":\"ask_llm\""));
    }

#[test]
fn empty_variant_serde() {
    let r = SkillResponse::empty();
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(json, r#"{"kind":"empty"}"#);
}

#[test]
fn sampled_pcm_variant_serde() {
    let r = SkillResponse::sampled_pcm(48000, vec![0.0, 0.5, -0.5]);
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains(r#""kind":"sampled_pcm"#));
    assert!(json.contains(r#""sample_rate":48000"#));
    assert!(json.contains("0.0"));
    let back: SkillResponse = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SkillResponse::SampledPcm { sample_rate, samples } if sample_rate == 48000 && samples.len() == 3));
}

#[test]
fn sampled_opus_variant_serde() {
    let r = SkillResponse::sampled_opus(vec![0x00, 0xFF, 0x7F]);
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains(r#""kind":"sampled_opus"#));
    let back: SkillResponse = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SkillResponse::SampledOpus { opus_frames } if opus_frames.len() == 3));
}

#[test]
fn volume_variant_serde() {
    let r = SkillResponse::volume(1.2);
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains(r#""kind":"volume"#));
    assert!(json.contains("1.2"));
    let back: SkillResponse = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SkillResponse::Volume(v) if (v - 1.2).abs() < f32::EPSILON));
    // Volume is clamped
    assert!(matches!(SkillResponse::volume(2.0), SkillResponse::Volume(1.5)));
}

    #[test]
    fn error_display() {
        assert!(
            SkillError::HttpFailed("boom".into())
                .to_string()
                .contains("boom")
        );
    }
}
