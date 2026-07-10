use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use athena_voice_core::ids::Locale;

use crate::error::RuntimeError;

#[derive(Debug, Clone, Deserialize)]
pub struct LocalePack {
    pub locale: Locale,
    pub llm_system_prompt: String,
    pub error_phrases: HashMap<String, String>,
}

pub fn load_pack(path: &Path) -> Result<LocalePack, RuntimeError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| RuntimeError::Locale(format!("read {}: {e}", path.display())))?;
    let pack: LocalePack = toml::from_str(&text)
        .map_err(|e| RuntimeError::Locale(format!("parse {}: {e}", path.display())))?;
    if pack.llm_system_prompt.is_empty() {
        return Err(RuntimeError::Locale(format!(
            "{}: llm_system_prompt must not be empty",
            path.display()
        )));
    }
    for required in [
        "stt_unavailable",
        "llm_unavailable",
        "tts_unavailable",
        "overloaded",
    ] {
        if !pack.error_phrases.contains_key(required) {
            return Err(RuntimeError::Locale(format!(
                "{}: missing required error_phrases.{required}",
                path.display()
            )));
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn workspace_root() -> PathBuf {
        // manifest_dir = crates/athena-voice-runtime/ so parent x2 = workspace root
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p
    }

    #[test]
    fn loads_fr_pack() {
        let path = workspace_root().join("locales/fr.toml");
        let pack = load_pack(&path).expect("load fr");
        assert_eq!(pack.locale.as_str(), "fr");
        assert!(pack.error_phrases.contains_key("stt_unavailable"));
    }

    #[test]
    fn loads_en_pack() {
        let path = workspace_root().join("locales/en.toml");
        let pack = load_pack(&path).expect("load en");
        assert_eq!(pack.locale.as_str(), "en");
        assert!(pack.error_phrases.contains_key("llm_unavailable"));
    }

    #[test]
    fn missing_required_key_rejected() {
        use std::io::Write;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
locale = "fr"
llm_system_prompt = "hi"

[error_phrases]
timeout = "boom"
            "#
        )
        .unwrap();
        let err = load_pack(f.path()).unwrap_err();
        assert!(matches!(err, RuntimeError::Locale(_)));
    }
}
