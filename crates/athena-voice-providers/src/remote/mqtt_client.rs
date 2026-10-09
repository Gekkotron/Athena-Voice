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
    /// Constructs a client, connects to the broker (queued), and spawns the
    /// pump task, which subscribes to `response_topic` on every connect.
    // Stays `async` so callers keep their `.await`; `spawn_pump` needs a
    // runtime context to `tokio::spawn`.
    #[allow(clippy::unused_async)]
    pub async fn connect(
        broker_host: impl Into<String>,
        broker_port: u16,
        client_id: impl Into<String>,
        request_topic: impl Into<String>,
        response_topic: impl Into<String>,
        request_timeout: Duration,
    ) -> Self {
        let mut opts = MqttOptions::new(client_id.into(), broker_host.into(), broker_port);
        // Streaming TTS responses carry base64 PCM chunks (~12 KiB of JSON
        // per 200 ms) — well above rumqttc's 10 KiB default packet cap.
        opts.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
        let (client, event_loop) = AsyncClient::new(opts, 128);
        let response_topic = response_topic.into();
        // `response_topic` is (re)subscribed by the pump on every `ConnAck`
        // — that fires on the first connect too, so there is no separate
        // startup subscribe here.
        let routes: Arc<DashMap<SessionId, mpsc::Sender<Publish>>> = Arc::new(DashMap::new());
        let pump = spawn_pump(
            client.clone(),
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
            .publish(
                &self.request_topic,
                QoS::AtLeastOnce,
                false,
                payload.to_vec(),
            )
            .await
            .map_err(|e| format!("mqtt publish: {e}"))?;
        Ok(rx)
    }

    /// Convenience: wait for exactly one response (or timeout).
    pub async fn call_once(&self, session: SessionId, payload: Bytes) -> Result<Publish, String> {
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

    /// Timeout the caller should apply while waiting for streamed responses.
    #[must_use]
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Publishes a follow-up message on the request topic without touching
    /// the response routing (used for streaming request bodies, e.g. STT
    /// audio frames).
    pub async fn publish_request(&self, payload: Bytes) -> Result<(), String> {
        self.client
            .publish(
                &self.request_topic,
                QoS::AtLeastOnce,
                false,
                payload.to_vec(),
            )
            .await
            .map_err(|e| format!("mqtt publish: {e}"))
    }
}

/// Pumps the MQTT event loop and routes incoming publishes on `response_topic`
/// to the appropriate per-session channel. Subscribes to `response_topic` on
/// every `ConnAck`: rumqttc 0.24's `MqttState::clean()` re-queues only
/// Publish/PubRel on reconnect, NOT prior Subscribe packets, so a one-shot
/// startup subscribe would silently die with the first broker restart.
fn spawn_pump(
    client: AsyncClient,
    event_loop: Arc<Mutex<EventLoop>>,
    routes: Arc<DashMap<SessionId, mpsc::Sender<Publish>>>,
    response_topic: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let event = { event_loop.lock().await.poll().await };
            match event {
                Ok(rumqttc::Event::Incoming(rumqttc::Incoming::ConnAck(_))) => {
                    if let Err(e) = client.subscribe(&response_topic, QoS::AtLeastOnce).await {
                        warn!(error = %e, "mqtt provider client subscribe failed");
                    }
                }
                Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(p))) => {
                    if p.topic != response_topic {
                        continue;
                    }
                    let Some(sid) = extract_session(&p.payload) else {
                        continue;
                    };
                    let closed = routes.get(&sid).is_some_and(|sender| {
                        matches!(
                            sender.try_send(p),
                            Err(mpsc::error::TrySendError::Closed(_))
                        )
                    });
                    // Receiver dropped (stream finished / timed out): drop the
                    // stale route so the map doesn't grow per session.
                    if closed {
                        routes.remove(&sid);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads one MQTT packet: fixed-header byte plus the body, framed by the
    /// remaining-length varint.
    async fn read_packet(s: &mut tokio::net::TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let first = s.read_u8().await?;
        let (mut len, mut mult) = (0usize, 1usize);
        loop {
            let b = s.read_u8().await?;
            len += usize::from(b & 0x7f) * mult;
            if b & 0x80 == 0 {
                break;
            }
            mult *= 128;
        }
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await?;
        Ok((first, body))
    }

    /// Reads packets until a SUBSCRIBE arrives; returns (packet id, topic).
    /// Anything else (PINGREQ, ...) is ignored.
    async fn read_subscribe(s: &mut tokio::net::TcpStream) -> std::io::Result<(u16, String)> {
        loop {
            let (first, body) = read_packet(s).await?;
            if first == 0x82 {
                let pid = u16::from_be_bytes([body[0], body[1]]);
                let tlen = usize::from(u16::from_be_bytes([body[2], body[3]]));
                let topic = String::from_utf8_lossy(&body[4..4 + tlen]).into_owned();
                return Ok((pid, topic));
            }
        }
    }

    /// Minimal MQTT 3.1.1 broker: the first connection is accepted, its
    /// SUBSCRIBE acked, then the socket is dropped (a broker restart). The
    /// task resolves with the topic subscribed on the second connection.
    async fn fake_broker_restart() -> (u16, tokio::task::JoinHandle<String>) {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            assert_eq!(read_packet(&mut s).await.unwrap().0, 0x10, "CONNECT");
            s.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
            let (pid, _) = read_subscribe(&mut s).await.unwrap();
            let [hi, lo] = pid.to_be_bytes();
            s.write_all(&[0x90, 0x03, hi, lo, 0x01]).await.unwrap();
            drop(s);

            let (mut s, _) = listener.accept().await.unwrap();
            assert_eq!(read_packet(&mut s).await.unwrap().0, 0x10, "CONNECT");
            s.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
            read_subscribe(&mut s).await.unwrap().1
        });
        (port, task)
    }

    /// rumqttc does not replay SUBSCRIBE packets on reconnect, so the pump
    /// must subscribe on every `ConnAck` or responses stop arriving after a
    /// broker restart.
    #[tokio::test]
    async fn resubscribes_response_topic_after_broker_restart() {
        let (port, broker) = fake_broker_restart().await;
        let _client = MqttProviderClient::connect(
            "127.0.0.1",
            port,
            "test-client",
            "athena/providers/x/request",
            "athena/providers/x/response",
            Duration::from_secs(1),
        )
        .await;

        let topic = tokio::time::timeout(Duration::from_secs(5), broker)
            .await
            .expect("no SUBSCRIBE after reconnect")
            .unwrap();
        assert_eq!(topic, "athena/providers/x/response");
    }
}
