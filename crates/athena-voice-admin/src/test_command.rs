//! Test console: drives a one-shot text session over MQTT, the same
//! satellite topic contract an Android app or `athena-voice-client` uses
//! (see `crates/athena-voice-runtime/src/mqtt/topics.rs`).

use std::time::Duration;

/// Broker coordinates for the test console, mirroring the CLI's `[mqtt]`
/// config. `None` in `AdminDeps.mqtt` disables the endpoint (503).
#[derive(Clone, Debug)]
pub struct AdminMqttConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

// TODO(next commit): allow removed when the handler reads the message.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum TestCommandError {
    /// Broker unreachable / protocol error → 502.
    Connect(String),
    /// No answer before the deadline → 504.
    Timeout,
}

/// LLM streaming can flush several `tts/text` segments; a quiet gap after
/// the first one is treated as end-of-answer (same heuristic as
/// `athena-voice-client`).
const ANSWER_QUIET: Duration = Duration::from_millis(1200);
const SESSION_TIMEOUT: Duration = Duration::from_secs(10);

// TODO(next commit): the allow goes away when the /api/test-command
// handler starts calling this — until then the non-test lib build (built
// for the integration-test target) has no caller.
#[allow(dead_code)]
pub(crate) async fn run_text_session(
    cfg: &AdminMqttConfig,
    text: &str,
    locale: &str,
) -> Result<String, TestCommandError> {
    use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};

    let sid = uuid::Uuid::new_v4();
    let base = format!("athena/sat/admin-ui/session/{sid}");

    // Unique client id: concurrent test requests and the runtime's own
    // MQTT client must never collide on the broker.
    let mut opts = MqttOptions::new(
        format!("athena-admin-test-{}-{}", std::process::id(), &sid.to_string()[..8]),
        &cfg.host,
        cfg.port,
    );
    opts.set_keep_alive(Duration::from_secs(15));
    // TTS audio chunks (~9 KiB PCM) arrive on our wildcard subscription —
    // rumqttc's 10 KiB default cap is too close.
    opts.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
    if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
        opts.set_credentials(u, p);
    }
    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    client
        .subscribe(format!("{base}/#"), QoS::AtLeastOnce)
        .await
        .map_err(|e| TestCommandError::Connect(e.to_string()))?;

    let deadline = tokio::time::Instant::now() + SESSION_TIMEOUT;
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut started = false;
    let mut segments: Vec<String> = Vec::new();
    let mut last_segment_at = tokio::time::Instant::now();

    let result = loop {
        let ev = tokio::select! {
            ev = eventloop.poll() => ev,
            _ = tick.tick() => {
                if !segments.is_empty() && last_segment_at.elapsed() > ANSWER_QUIET {
                    break Ok(segments.join(" "));
                }
                continue;
            }
            () = tokio::time::sleep_until(deadline) => {
                break if segments.is_empty() {
                    Err(TestCommandError::Timeout)
                } else {
                    Ok(segments.join(" "))
                };
            }
        };
        match ev {
            Ok(MqttEvent::Incoming(Packet::SubAck(_))) if !started => {
                // Subscription is live — safe to open the session now.
                started = true;
                let start = client
                    .publish(
                        format!("{base}/start"),
                        QoS::AtLeastOnce,
                        false,
                        serde_json::json!({ "locale": locale }).to_string(),
                    )
                    .await;
                let text_pub = client
                    .publish(format!("{base}/text"), QoS::AtLeastOnce, false, text.to_string())
                    .await;
                if let Err(e) = start.and(text_pub) {
                    break Err(TestCommandError::Connect(e.to_string()));
                }
            }
            Ok(MqttEvent::Incoming(Packet::Publish(p))) => {
                if p.topic == format!("{base}/tts/text") {
                    segments.push(String::from_utf8_lossy(&p.payload).to_string());
                    last_segment_at = tokio::time::Instant::now();
                } else if p.topic == format!("{base}/done") {
                    break Ok(segments.join(" "));
                }
            }
            Ok(_) => {}
            Err(e) => break Err(TestCommandError::Connect(e.to_string())),
        }
    };

    // Close the session server-side even on failure — otherwise it lingers
    // until the session manager's idle reaper. Publishing only ENQUEUES;
    // poll until the broker acks (or give up after 1 s), then disconnect.
    let _ = client
        .publish(format!("{base}/end"), QoS::AtLeastOnce, false, "")
        .await;
    let flush_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        tokio::select! {
            ev = eventloop.poll() => match ev {
                Ok(MqttEvent::Incoming(Packet::PubAck(_))) | Err(_) => break,
                Ok(_) => {}
            },
            () = tokio::time::sleep_until(flush_deadline) => break,
        }
    }
    let _ = client.disconnect().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_broker_reports_connect_error() {
        let cfg = AdminMqttConfig {
            host: "127.0.0.1".into(),
            port: 1, // nothing listens here — refused immediately
            username: None,
            password: None,
        };
        let err = run_text_session(&cfg, "hello", "en").await.unwrap_err();
        assert!(matches!(err, TestCommandError::Connect(_)));
    }
}
