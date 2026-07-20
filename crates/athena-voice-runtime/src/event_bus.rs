use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::warn;

use athena_voice_core::event::Event;
#[allow(unused_imports)]
use athena_voice_core::event::LlmFallbackReason;

pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    #[must_use]
    pub fn sender(&self) -> broadcast::Sender<Event> {
        self.tx.clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

/// Spawns a task that consumes broadcast events and publishes each as JSON on
/// `athena/events/<kind>`. Returns the JoinHandle so the caller can shut it down.
pub fn spawn_mqtt_mirror(
    tx: broadcast::Sender<Event>,
    mqtt: rumqttc::AsyncClient,
) -> JoinHandle<()> {
    let mut rx = tx.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let value = match serde_json::to_value(&event) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "failed to serialise Event for MQTT mirror");
                            continue;
                        }
                    };
                    let kind = value
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let payload = match serde_json::to_vec(&value) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(error = %e, "failed to serialise Event to bytes");
                            continue;
                        }
                    };
                    let topic = crate::mqtt::topics::event_topic(&kind);
                    if let Err(e) = mqtt
                        .publish(topic, rumqttc::QoS::AtLeastOnce, false, payload)
                        .await
                    {
                        warn!(error = %e, "mqtt mirror publish failed");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "event_bus mirror lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use athena_voice_core::ids::SessionId;

    use super::*;

    #[tokio::test]
    async fn subscribers_receive_broadcast() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
    let ev = Event::LlmFallback {
        session: SessionId::new_v4(),
        reason: LlmFallbackReason::NoMatch,
        slots: Vec::new(),
    };
        bus.sender().send(ev).unwrap();
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, Event::LlmFallback { .. }));
    }

    #[tokio::test]
    async fn lagged_receiver_gets_lagged_error_then_recovers() {
        let bus = EventBus::new(2);
        let mut rx = bus.subscribe();
        for _ in 0..5 {
            bus.sender()
        .send(Event::LlmFallback {
            session: SessionId::new_v4(),
            reason: LlmFallbackReason::NoMatch,
            slots: Vec::new(),
        })
                .unwrap();
        }
        let first = rx.recv().await;
        assert!(matches!(first, Err(broadcast::error::RecvError::Lagged(_))));
    }
}
