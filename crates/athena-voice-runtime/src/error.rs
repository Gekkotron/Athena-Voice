use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("mqtt: {0}")]
    Mqtt(#[from] rumqttc::ConnectionError),
    #[error("mqtt client: {0}")]
    MqttClient(#[from] rumqttc::ClientError),
    #[error("locale pack: {0}")]
    Locale(String),
    #[error("config: {0}")]
    Config(String),
    #[error("shutdown")]
    Shutdown,
}
