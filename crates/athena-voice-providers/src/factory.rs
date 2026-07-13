use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use athena_voice_core::provider::{Llm, Stt, Tts};

use crate::circuit::CircuitBreaker;
use crate::remote::{MqttStt, MqttTts, OllamaLlm};
use crate::retry::{RetryConfig, RetryingLlm, RetryingStt, RetryingTts};
use crate::testing::{fake_llm::FakeLlm, fake_stt::FakeStt, fake_tts::FakeTts};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub stt: StageChoice,
    pub llm: StageChoice,
    pub tts: StageChoice,
}

/// Which concrete provider backs a stage. Serialised as tagged enum:
///
/// ```toml
/// stt = "fake"
/// llm = { ollama = { base_url = "http://localhost:11434", model = "llama3.2:3b" } }
/// tts = { mqtt_tts = { name = "piper" } }
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageChoice {
    /// Deterministic fake — used by tests and as the default in `athena.example.toml`.
    Fake,
    /// Real LLM via Ollama HTTP.
    Ollama { base_url: String, model: String },
    /// STT provider that speaks the generic MQTT protocol under `athena/providers/stt/<name>/…`.
    MqttStt { name: String },
    /// TTS provider that speaks the generic MQTT protocol under `athena/providers/tts/<name>/…`.
    MqttTts { name: String },
}

/// Shared configuration needed to construct MQTT-based providers.
#[derive(Debug, Clone)]
pub struct MqttBrokerAddr {
    pub host: String,
    pub port: u16,
}

pub struct ProviderFactory {
    stt: Arc<dyn Stt>,
    llm: Arc<dyn Llm>,
    tts: Arc<dyn Tts>,
}

impl ProviderFactory {
    /// Constructs the factory. If any stage requires MQTT (either MqttStt or
    /// MqttTts), `broker` must be `Some`.
    ///
    /// # Errors
    /// Returns an error string if an MQTT variant is chosen without a broker
    /// address, or if a variant name is not supported (all name-based STT/TTS
    /// providers can be looked up as `&'static str` by leaking a small set of
    /// well-known names).
    pub async fn build(
        config: &ProviderConfig,
        broker: Option<&MqttBrokerAddr>,
    ) -> Result<Self, String> {
        let stt = build_stt(&config.stt, broker).await?;
        let llm = build_llm(&config.llm)?;
        let tts = build_tts(&config.tts, broker).await?;

        // Every provider is wrapped in its retry/circuit decorator with sensible defaults.
        let stt = Arc::new(RetryingStt::new(stt, fresh_circuit())) as Arc<dyn Stt>;
        let llm = Arc::new(RetryingLlm::new(llm, fresh_circuit())) as Arc<dyn Llm>;
        let tts =
            Arc::new(RetryingTts::new(tts, fresh_circuit(), RetryConfig::tts())) as Arc<dyn Tts>;

        Ok(Self { stt, llm, tts })
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

fn fresh_circuit() -> Arc<CircuitBreaker> {
    Arc::new(CircuitBreaker::new(
        5,
        Duration::from_secs(60),
        Duration::from_secs(15),
    ))
}

async fn build_stt(
    choice: &StageChoice,
    broker: Option<&MqttBrokerAddr>,
) -> Result<Arc<dyn Stt>, String> {
    match choice {
        StageChoice::Fake => Ok(Arc::new(FakeStt::builder().build())),
        StageChoice::MqttStt { name } => {
            let broker = broker.ok_or("MqttStt requires an [mqtt] broker address")?;
            // Leak a static string so the `Stt::name()` API can return &'static str.
            let leaked: &'static str = Box::leak(name.clone().into_boxed_str());
            Ok(Arc::new(
                MqttStt::connect(&broker.host, broker.port, leaked).await,
            ))
        }
        StageChoice::Ollama { .. } | StageChoice::MqttTts { .. } => {
            Err(format!("cannot use {choice:?} as STT provider"))
        }
    }
}

fn build_llm(choice: &StageChoice) -> Result<Arc<dyn Llm>, String> {
    match choice {
        StageChoice::Fake => Ok(Arc::new(FakeLlm::builder().build())),
        StageChoice::Ollama { base_url, model } => {
            Ok(Arc::new(OllamaLlm::new(base_url.clone(), model.clone())))
        }
        StageChoice::MqttStt { .. } | StageChoice::MqttTts { .. } => {
            Err(format!("cannot use {choice:?} as LLM provider"))
        }
    }
}

async fn build_tts(
    choice: &StageChoice,
    broker: Option<&MqttBrokerAddr>,
) -> Result<Arc<dyn Tts>, String> {
    match choice {
        StageChoice::Fake => Ok(Arc::new(FakeTts::new())),
        StageChoice::MqttTts { name } => {
            let broker = broker.ok_or("MqttTts requires an [mqtt] broker address")?;
            let leaked: &'static str = Box::leak(name.clone().into_boxed_str());
            Ok(Arc::new(
                MqttTts::connect(&broker.host, broker.port, leaked).await,
            ))
        }
        StageChoice::Ollama { .. } | StageChoice::MqttStt { .. } => {
            Err(format!("cannot use {choice:?} as TTS provider"))
        }
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
    fn ollama_config_roundtrips_toml() {
        let toml_input = r#"
stt = "fake"
tts = "fake"

[llm.ollama]
base_url = "http://localhost:11434"
model = "llama3.2:3b"
        "#;
        let cfg: ProviderConfig = toml::from_str(toml_input).unwrap();
        match cfg.llm {
            StageChoice::Ollama { base_url, model } => {
                assert_eq!(base_url, "http://localhost:11434");
                assert_eq!(model, "llama3.2:3b");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn mqtt_stt_config_roundtrips_toml() {
        let toml_input = r#"
llm = "fake"
tts = "fake"

[stt.mqtt_stt]
name = "whisper"
        "#;
        let cfg: ProviderConfig = toml::from_str(toml_input).unwrap();
        assert!(matches!(cfg.stt, StageChoice::MqttStt { .. }));
    }

    #[tokio::test]
    async fn factory_returns_named_providers_for_fake() {
        let cfg = ProviderConfig {
            stt: StageChoice::Fake,
            llm: StageChoice::Fake,
            tts: StageChoice::Fake,
        };
        let f = ProviderFactory::build(&cfg, None).await.unwrap();
        // Names are transparent through the retry decorator.
        assert_eq!(f.stt().name(), "fake-stt");
        assert_eq!(f.llm().name(), "fake-llm");
        assert_eq!(f.tts().name(), "fake-tts");
    }

    #[tokio::test]
    async fn factory_rejects_mqtt_stt_without_broker() {
        let cfg = ProviderConfig {
            stt: StageChoice::MqttStt {
                name: "whisper".into(),
            },
            llm: StageChoice::Fake,
            tts: StageChoice::Fake,
        };
        assert!(ProviderFactory::build(&cfg, None).await.is_err());
    }
}
