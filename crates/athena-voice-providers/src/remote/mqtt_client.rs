//! Generic MQTT request/reply client for language-agnostic providers.
//!
//! Each `MqttProviderClient` owns its own [`rumqttc::AsyncClient`] connection to
//! the broker. On construction it spawns a background task that pumps the event
//! loop and dispatches incoming publishes to per-session response channels based
//! on the `session_id` field in the JSON payload.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, Publish, QoS};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::warn;

use athena_voice_core::ids::SessionId;

pub struct MqttProviderClient {
    client: AsyncClient,
    routes: Arc<DashMap<SessionId, mpsc::Sender<Publish>>>,
    request_topic: String,
    response_topic: String,
    request_timeout: Duration,
    _pump: Arc<JoinHandle<()>>,
}

impl MqttProviderClient {
    /// Constructs a client, connects to the broker (queued), subscribes to
    /// `response_topic`, and spawns the pump task.
    pub async fn connect(
        broker_host: impl Into<String>,
        broker_port: u16,
        client_id: impl Into<String>,
        request_topic: impl Into<String>,
        response_topic: impl Into<String>,
        request_timeout: Duration,
    ) -> Self {
        let opts = MqttOptions::new(client_id.into(), broker_host.into(), broker_port);
        let (client, event_loop) = AsyncClient::new(opts, 128);
        let response_topic = response_topic.into();
        // Subscribe to responses. Ignore error — pump will retry on reconnect.
        let _ = client.subscribe(&response_topic, QoS::AtLeastOnce).await;

        let routes: Arc<DashMap<SessionId, mpsc::Sender<Publish>>> = Arc::new(DashMap::new());
        let pump = spawn_pump(
            Arc::new(Mutex::new(event_loop)),
            routes.clone(),
            response_topic.clone(),
        );

        Self {
            client,
            routes,
            request_topic: request_topic.into(),
            response_topic,
            request_timeout,
            _pump: Arc::new(pump),
        }
    }

    /// Publishes a request tagged with `session` and returns a channel that
    /// yields each response publish routed to this session.
    pub async fn call_streaming(
        &self,
        session: SessionId,
        payload: Bytes,
    ) -> Result<mpsc::Receiver<Publish>, String> {
        let (tx, rx) = mpsc::channel::<Publish>(32);
        self.routes.insert(session, tx);
        self.client
            .publish(&self.request_topic, QoS::AtLeastOnce, false, payload.to_vec())
            .await
            .map_err(|e| format!("mqtt publish: {e}"))?;
        Ok(rx)
    }

    /// Convenience: wait for exactly one response (or timeout).
    pub async fn call_once(
        &self,
        session: SessionId,
        payload: Bytes,
    ) -> Result<Publish, String> {
        let mut rx = self.call_streaming(session, payload).await?;
        match tokio::time::timeout(self.request_timeout, rx.recv()).await {
            Ok(Some(p)) => Ok(p),
            Ok(None) => Err("mqtt response channel closed".into()),
            Err(_) => Err("mqtt request timed out".into()),
        }
    }

    #[must_use]
    pub fn response_topic(&self) -> &str {
        &self.response_topic
    }
}

/// Pumps the MQTT event loop and routes incoming publishes on `response_topic`
/// to the appropriate per-session channel.
fn spawn_pump(
    event_loop: Arc<Mutex<EventLoop>>,
    routes: Arc<DashMap<SessionId, mpsc::Sender<Publish>>>,
    response_topic: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let event = { event_loop.lock().await.poll().await };
            match event {
                Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(p))) => {
                    if p.topic != response_topic {
                        continue;
                    }
                    let Some(sid) = extract_session(&p.payload) else {
                        continue;
                    };
                    if let Some(sender) = routes.get(&sid) {
                        let _ = sender.try_send(p);
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(error = %err, "mqtt provider client pump error");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    })
}

fn extract_session(payload: &[u8]) -> Option<SessionId> {
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    v.get("session_id")?.as_str()?.parse().ok()
}
