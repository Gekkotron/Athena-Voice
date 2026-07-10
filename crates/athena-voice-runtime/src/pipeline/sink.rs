use bytes::Bytes;
use rumqttc::{AsyncClient, QoS};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::{Event, Outcome};
use athena_voice_core::ids::{SatelliteId, SessionId};

use crate::mqtt::topics;

/// ResponseSink: consumes TTS chunks and publishes them (plus a leading `tts/meta`
/// and a trailing `done`) to the satellite egress topics.
pub fn spawn_sink(
    session: SessionId,
    sat: SatelliteId,
    mqtt: AsyncClient,
    mut chunk_rx: mpsc::Receiver<Bytes>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sent_meta = false;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = chunk_rx.recv() => match maybe {
                    Some(chunk) => {
                        if !sent_meta {
                            let meta = json!({
                                "sample_rate": 24000,
                                "channels": 1,
                                "frame_ms": 20,
                                "codec": "opus"
                            });
                            if let Err(e) = mqtt
                                .publish(
                                    topics::session_tts_meta(&sat, session),
                                    QoS::AtLeastOnce,
                                    false,
                                    meta.to_string(),
                                )
                                .await
                            {
                                warn!(error = %e, "sink tts/meta publish failed");
                            }
                            sent_meta = true;
                        }
                        if let Err(e) = mqtt
                            .publish(
                                topics::session_tts(&sat, session),
                                QoS::AtMostOnce,
                                false,
                                chunk.to_vec(),
                            )
                            .await
                        {
                            warn!(error = %e, "sink tts chunk publish failed");
                        }
                    }
                    None => break,
                }
            }
        }
        // Publish done + emit SessionEnded.
        let done = json!({ "outcome": "ok" });
        if let Err(e) = mqtt
            .publish(
                topics::session_done(&sat, session),
                QoS::AtLeastOnce,
                false,
                done.to_string(),
            )
            .await
        {
            warn!(error = %e, "sink done publish failed");
        }
        let _ = event_tx.send(Event::SessionEnded {
            session,
            outcome: Outcome::Ok,
        });
    })
}
