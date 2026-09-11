use bytes::Bytes;
use rumqttc::{AsyncClient, QoS};
use serde_json::json;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::{AudioFormat, Event, Outcome};
use athena_voice_core::ids::{SatelliteId, SessionId};

use crate::mqtt::topics;

/// The `tts/meta` payload satellites read to configure playback. It
/// reports the provider's actual format — a wrong value here means
/// every satellite plays noise.
fn meta_payload(format: AudioFormat, sample_rate: u32) -> serde_json::Value {
    json!({
        "codec": format,
        "sample_rate": sample_rate,
        "channels": 1,
    })
}

/// What the TTS stage feeds the sink: the provider's declared format
/// (sent once per synthesize call, before its chunks), then raw chunks.
#[derive(Debug)]
pub enum SinkMsg {
    Format { format: AudioFormat, sample_rate: u32 },
    Chunk(Bytes),
}

/// ResponseSink: consumes TTS chunks and publishes them (plus a leading `tts/meta`
/// carrying the provider's real format, and a trailing `done`) to the
/// satellite egress topics.
pub fn spawn_sink(
    session: SessionId,
    sat: SatelliteId,
    topic_root: std::sync::Arc<str>,
    mqtt: AsyncClient,
    mut chunk_rx: mpsc::Receiver<SinkMsg>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sent: Option<(AudioFormat, u32)> = None;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = chunk_rx.recv() => match maybe {
                    Some(SinkMsg::Format { format, sample_rate }) => {
                        match sent {
                            None => {
                                let meta = meta_payload(format, sample_rate);
                                if let Err(e) = mqtt
                                    .publish(
                                        topics::session_tts_meta(&topic_root, &sat, session),
                                        QoS::AtLeastOnce,
                                        false,
                                        meta.to_string(),
                                    )
                                    .await
                                {
                                    warn!(error = %e, "sink tts/meta publish failed");
                                }
                                sent = Some((format, sample_rate));
                            }
                            // Meta is published once per session; a provider
                            // changing format mid-session would garble
                            // playback, so surface it loudly.
                            Some(prev) if prev != (format, sample_rate) => {
                                warn!(
                                    ?prev, now = ?(format, sample_rate),
                                    "tts format changed mid-session; satellites keep the first meta"
                                );
                            }
                            Some(_) => {}
                        }
                    }
                    Some(SinkMsg::Chunk(chunk)) => {
                        if let Err(e) = mqtt
                            .publish(
                                topics::session_tts(&topic_root, &sat, session),
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
                topics::session_done(&topic_root, &sat, session),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_reports_the_providers_real_format() {
        // The say/Piper workers stream s16le at their own rate — the
        // metadata must say so (this used to be hardcoded opus/24000).
        let meta = meta_payload(AudioFormat::S16le, 22_050);
        assert_eq!(meta["codec"], "s16le");
        assert_eq!(meta["sample_rate"], 22_050);
        assert_eq!(meta["channels"], 1);
    }

    #[test]
    fn meta_carries_each_format_verbatim() {
        for (format, codec) in [
            (AudioFormat::Opus, "opus"),
            (AudioFormat::F32le, "f32le"),
            (AudioFormat::Text, "text"),
        ] {
            assert_eq!(meta_payload(format, 48_000)["codec"], codec);
        }
    }
}
