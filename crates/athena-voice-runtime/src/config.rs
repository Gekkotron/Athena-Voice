use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub mqtt: crate::mqtt::MqttConfig,
}
