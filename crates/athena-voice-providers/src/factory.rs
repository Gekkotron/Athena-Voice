use std::sync::Arc;

use serde::{Deserialize, Serialize};

use athena_voice_core::provider::{Llm, Stt, Tts};

use crate::testing::{fake_llm::FakeLlm, fake_stt::FakeStt, fake_tts::FakeTts};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub stt: StageChoice,
    pub llm: StageChoice,
    pub tts: StageChoice,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageChoice {
    Fake,
    // Ollama, MqttRpc, Whisper, Piper — added in Plan 3
}

pub struct ProviderFactory {
    stt: Arc<dyn Stt>,
    llm: Arc<dyn Llm>,
    tts: Arc<dyn Tts>,
}

impl ProviderFactory {
    #[must_use]
    pub fn new(config: &ProviderConfig) -> Self {
        let stt: Arc<dyn Stt> = match config.stt {
            StageChoice::Fake => Arc::new(FakeStt::builder().build()),
        };
        let llm: Arc<dyn Llm> = match config.llm {
            StageChoice::Fake => Arc::new(FakeLlm::builder().build()),
        };
        let tts: Arc<dyn Tts> = match config.tts {
            StageChoice::Fake => Arc::new(FakeTts::new()),
        };
        Self { stt, llm, tts }
    }

    #[must_use]
    pub fn with_stt(mut self, stt: Arc<dyn Stt>) -> Self {
        self.stt = stt;
        self
    }

    #[must_use]
    pub fn with_llm(mut self, llm: Arc<dyn Llm>) -> Self {
        self.llm = llm;
        self
    }

    #[must_use]
    pub fn with_tts(mut self, tts: Arc<dyn Tts>) -> Self {
        self.tts = tts;
        self
    }

    #[must_use]
    pub fn stt(&self) -> Arc<dyn Stt> {
        self.stt.clone()
    }
    #[must_use]
    pub fn llm(&self) -> Arc<dyn Llm> {
        self.llm.clone()
    }
    #[must_use]
    pub fn tts(&self) -> Arc<dyn Tts> {
        self.tts.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fake_config_roundtrips_toml() {
        let toml_input = r#"
stt = "fake"
llm = "fake"
tts = "fake"
        "#;
        let cfg: ProviderConfig = toml::from_str(toml_input).unwrap();
        assert!(matches!(cfg.stt, StageChoice::Fake));
        assert!(matches!(cfg.llm, StageChoice::Fake));
        assert!(matches!(cfg.tts, StageChoice::Fake));
    }

    #[test]
    fn factory_returns_named_providers() {
        let cfg = ProviderConfig {
            stt: StageChoice::Fake,
            llm: StageChoice::Fake,
            tts: StageChoice::Fake,
        };
        let f = ProviderFactory::new(&cfg);
        assert_eq!(f.stt().name(), "fake-stt");
        assert_eq!(f.llm().name(), "fake-llm");
        assert_eq!(f.tts().name(), "fake-tts");
    }
}
