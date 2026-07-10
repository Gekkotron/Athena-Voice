use thiserror::Error;

#[derive(Debug, Error)]
pub enum SttError {
    #[error("stt provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("stt provider {name} unavailable: {reason}")]
    Unavailable { name: &'static str, reason: String },
    #[error("bad audio: {0}")]
    BadAudio(String),
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl SttError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::Unavailable { .. })
    }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::BadAudio(_) => "BadAudio",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("llm provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("llm provider {name} unavailable: {reason}")]
    Unavailable { name: &'static str, reason: String },
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    /// LLM is nondeterministic — retrying can produce a different answer. Never auto-retry.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        false
    }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("tts provider {name} timed out after {ms}ms")]
    Timeout { name: &'static str, ms: u64 },
    #[error("tts provider {name} unavailable: {reason}")]
    Unavailable { name: &'static str, reason: String },
    #[error("bad audio: {0}")]
    BadAudio(String),
    #[error("circuit open, retry in {retry_after_ms}ms")]
    CircuitOpen { retry_after_ms: u64 },
    #[error("cancelled")]
    Cancelled,
}

impl TtsError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::Unavailable { .. })
    }
    #[must_use]
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "Timeout",
            Self::Unavailable { .. } => "Unavailable",
            Self::BadAudio(_) => "BadAudio",
            Self::CircuitOpen { .. } => "CircuitOpen",
            Self::Cancelled => "Cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_is_retryable_truth_table() {
        assert!(SttError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(SttError::Unavailable { name: "fake", reason: "boom".into() }.is_retryable());
        assert!(!SttError::BadAudio("bad".into()).is_retryable());
        assert!(!SttError::CircuitOpen { retry_after_ms: 60_000 }.is_retryable());
        assert!(!SttError::Cancelled.is_retryable());
    }

    #[test]
    fn llm_no_retry_by_default() {
        assert!(!LlmError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(!LlmError::Unavailable { name: "fake", reason: "boom".into() }.is_retryable());
    }

    #[test]
    fn tts_retryable_on_transient() {
        assert!(TtsError::Timeout { name: "fake", ms: 5000 }.is_retryable());
        assert!(TtsError::Unavailable { name: "fake", reason: "boom".into() }.is_retryable());
        assert!(!TtsError::Cancelled.is_retryable());
    }

    #[test]
    fn variant_names_are_stable_strings() {
        assert_eq!(SttError::Timeout { name: "x", ms: 0 }.variant_name(), "Timeout");
        assert_eq!(LlmError::Cancelled.variant_name(), "Cancelled");
        assert_eq!(TtsError::BadAudio(String::new()).variant_name(), "BadAudio");
    }
}
