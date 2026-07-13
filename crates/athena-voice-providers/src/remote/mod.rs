//! Remote provider implementations (HTTP for LLM, MQTT for STT/TTS).

pub mod mqtt_client;
pub mod mqtt_stt;
pub mod mqtt_tts;
pub mod ollama;

pub use mqtt_client::MqttProviderClient;
pub use mqtt_stt::MqttStt;
pub use mqtt_tts::MqttTts;
pub use ollama::OllamaLlm;
