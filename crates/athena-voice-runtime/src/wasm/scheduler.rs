//! Scheduler task — ticks every 1 s, pops due scheduled events from the
//! `Store`, publishes them via MQTT, and emits `Event::ScheduledFired`.
//!
//! For the timer skill specifically, the scheduler also decodes the
//! `{"seconds": N}` payload and emits `Event::SkillNotify` carrying the
//! expiration announce. A separate forwarder task
//! ([`spawn_skill_notify_forwarder`]) subscribes to the event bus and pushes
//! `SkillNotify.text` into the router's TTS token channel — this is the
//! least-invasive integration point: it avoids threading a new dependency
//! through `RouterDeps` just to support one notification path.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rumqttc::{AsyncClient, QoS};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_core::ids::SessionId;
use athena_voice_storage::Store;

pub struct SchedulerTask;

impl SchedulerTask {
    /// Spawns the ticking scheduler. `session` is the session used to tag
    /// `Event::SkillNotify` — in the current single-session runtime this is
    /// the active satellite session.
    pub fn spawn(
        store: Arc<dyn Store>,
        mqtt: AsyncClient,
        event_tx: broadcast::Sender<Event>,
        session: SessionId,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    _ = interval.tick() => {
                        let now_ms = Utc::now().timestamp_millis();
                        match store.pop_due_events(now_ms).await {
                            Ok(events) => {
                                for ev in events {
                                    // Publish over MQTT (best-effort).
                                    if let Err(e) = mqtt
                                        .publish(&ev.mqtt_topic, QoS::AtLeastOnce, false, ev.payload.clone())
                                        .await
                                    {
                                        warn!(topic = %ev.mqtt_topic, error = %e, "scheduler mqtt publish failed");
                                    }
                                    let _ = event_tx.send(Event::ScheduledFired {
                                        skill: ev.skill.clone(),
                                        id: ev.id,
                                    });
                                    // For the timer skill: decode {seconds} JSON and
                                    // emit a spoken-notification event.
                                    if ev.skill == "timer" {
                                        let text = timer_expiration_text(&ev.payload);
                                        let _ = event_tx.send(Event::SkillNotify {
                                            session,
                                            skill: ev.skill.clone(),
                                            text,
                                        });
                                    }
                                }
                            }
                            Err(e) => warn!(error = %e, "pop_due_events failed"),
                        }
                    }
                }
            }
        })
    }
}

fn timer_expiration_text(payload: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Payload {
        seconds: u64,
    }
    let seconds = serde_json::from_slice::<Payload>(payload)
        .map_or(0, |p| p.seconds);
    format!("le minuteur de {seconds} secondes est terminé.")
}

/// Subscribes to the event bus and forwards `Event::SkillNotify.text` into
/// the router's TTS token channel, bypassing the intent matcher entirely.
/// This is how skill-triggered speech (e.g. a fired timer) reaches TTS
/// without a matching user utterance.
pub fn spawn_skill_notify_forwarder(
    mut event_rx: broadcast::Receiver<Event>,
    tts_tok_tx: mpsc::Sender<String>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                ev = event_rx.recv() => match ev {
                    Ok(Event::SkillNotify { text, .. }) => {
                        if tts_tok_tx.send(text).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    })
}
