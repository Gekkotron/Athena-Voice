//! Remote provider implementations (HTTP for LLM, MQTT for STT/TTS).

pub mod ollama;

pub use ollama::OllamaLlm;
