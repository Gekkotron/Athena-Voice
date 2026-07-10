use thiserror::Error;

use crate::event::Stage;
use crate::ids::{IdError, Locale};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    InvalidId(#[from] IdError),

    #[error("cancelled")]
    Cancelled,

    #[error("stage {stage:?} timed out after {ms}ms")]
    Timeout { stage: Stage, ms: u64 },
}

impl CoreError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }

    #[must_use]
    pub fn to_user_message(&self, locale: &Locale) -> String {
        let lang = &locale.as_str()[..2];
        match (self, lang) {
            (Self::Timeout { .. }, "fr") => "Désolé, j'ai mis trop de temps à répondre.".into(),
            (Self::Timeout { .. }, _) => "Sorry, that timed out.".into(),
            (Self::Cancelled, "fr") => "Annulé.".into(),
            (Self::Cancelled, _) => "Cancelled.".into(),
            (Self::InvalidId(_), "fr") => "Identifiant invalide.".into(),
            (Self::InvalidId(_), _) => "Invalid identifier.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Stage;
    use crate::ids::{IdError, Locale};

    #[test]
    fn is_retryable_truth_table() {
        assert!(!CoreError::InvalidId(IdError::InvalidLocale("x".into())).is_retryable());
        assert!(!CoreError::Cancelled.is_retryable());
        assert!(CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.is_retryable());
    }

    #[test]
    fn user_message_fr() {
        let fr = Locale::new("fr").unwrap();
        let msg = CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.to_user_message(&fr);
        assert!(msg.contains("délai") || msg.contains("temps"), "got {msg}");
    }

    #[test]
    fn user_message_en() {
        let en = Locale::new("en").unwrap();
        let msg = CoreError::Timeout { stage: Stage::Stt, ms: 5000 }.to_user_message(&en);
        assert!(msg.contains("timed out") || msg.contains("timeout"), "got {msg}");
    }

    #[test]
    fn user_message_unknown_locale_falls_back_to_en() {
        let ja = Locale::new("ja").unwrap();
        let msg = CoreError::Cancelled.to_user_message(&ja);
        assert!(!msg.is_empty());
        assert!(msg.chars().all(|c| c.is_ascii()), "expected ASCII EN fallback, got: {msg}");
    }
}
