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
        Self::AskLlm { prompt: prompt.into() }
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
    fn error_display() {
        assert!(SkillError::HttpFailed("boom".into()).to_string().contains("boom"));
    }
}
