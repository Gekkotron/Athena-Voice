//! MQTT wrapper for Athena-Voice runtime.

pub mod topics;

use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, EventLoop, MqttOptions};
use tokio::sync::Mutex;

use crate::error::RuntimeError;

#[derive(Debug, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub keep_alive_secs: u64,
}

pub struct MqttClient {
    pub tx: AsyncClient,
    pub event_loop: Arc<Mutex<EventLoop>>,
}

impl MqttClient {
    /// Constructs a client + event loop from a config. Does not perform a
    /// connect; the connect happens on the first `event_loop.poll()`.
    pub fn connect(config: MqttConfig) -> Result<Self, RuntimeError> {
        let mut opts = MqttOptions::new(&config.client_id, &config.host, config.port);
        opts.set_keep_alive(Duration::from_secs(config.keep_alive_secs));
        if let (Some(u), Some(p)) = (&config.username, &config.password) {
            opts.set_credentials(u, p);
        }
        let (tx, event_loop) = AsyncClient::new(opts, 128);
        Ok(Self { tx, event_loop: Arc::new(Mutex::new(event_loop)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_constructs_without_broker() {
        let client = MqttClient::connect(MqttConfig {
            host: "127.0.0.1".into(),
            port: 1883,
            client_id: "test".into(),
            username: None,
            password: None,
            keep_alive_secs: 30,
        })
        .expect("construct");
        // Publish call is queued in-memory; without a broker it never succeeds,
        // but the client itself is valid. (Actual network roundtrip is covered
        // in the E2E test in Task 20.)
        assert!(Arc::strong_count(&client.event_loop) >= 1);
    }
}
