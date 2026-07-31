//! TTS provider that speaks the generic MQTT request/reply protocol.
//!
//! Wire format: request `{session_id, locale, text}` on `athena/providers/tts/<name>/request`.
//! Responses on `.../response` carry base64-encoded Opus chunks as
//! `{session_id, chunk_b64, done}`. Plan 3 uses the JSON envelope for simplicity;
//! a future revision can switch to a companion binary topic once bandwidth matters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde_json::json;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioStream, BoxError, Tts};

use super::mqtt_client::MqttProviderClient;

pub struct MqttTts {
    name: &'static str,
    client: Arc<MqttProviderClient>,
}

impl MqttTts {
    pub async fn connect(
        broker_host: impl Into<String>,
        broker_port: u16,
        provider_name: &'static str,
    ) -> Self {
        let request_topic = format!("athena/providers/tts/{provider_name}/request");
        let response_topic = format!("athena/providers/tts/{provider_name}/response");
        let client = MqttProviderClient::connect(
            broker_host,
            broker_port,
            format!("athena-voice-tts-{provider_name}"),
            request_topic,
            response_topic,
            Duration::from_secs(30),
        )
        .await;
        Self {
            name: provider_name,
            client: Arc::new(client),
        }
    }
}

#[async_trait]
impl Tts for MqttTts {
    async fn synthesize(
        &self,
        session: SessionId,
        locale: Locale,
        text: String,
    ) -> Result<AudioStream, BoxError> {
        let request = json!({
            "session_id": session.to_string(),
            "locale": locale.as_str(),
            "text": text,
        });
        let payload = Bytes::from(request.to_string().into_bytes());
        let rx = self.client.call_streaming(session, payload).await?;
        let timeout = self.client.request_timeout();

        // The stream must terminate: on the worker's `done: true` marker, on
        // a per-message timeout (worker died mid-stream), or on channel
        // close. A `done` message may itself carry a final chunk.
        let audio_stream =
            futures::stream::unfold((rx, false), move |(mut rx, finished)| async move {
                if finished {
                    return None;
                }
                loop {
                    match tokio::time::timeout(timeout, rx.recv()).await {
                        Err(_) | Ok(None) => return None,
                        Ok(Some(publish)) => {
                            let Ok(v) =
                                serde_json::from_slice::<serde_json::Value>(&publish.payload)
                            else {
                                continue;
                            };
                            let done = v
                                .get("done")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            let chunk = v
                                .get("chunk_b64")
                                .and_then(serde_json::Value::as_str)
                                .and_then(|b| STANDARD.decode(b).ok());
                            match chunk {
                                Some(bytes) => {
                                    return Some((
                                        Ok::<Bytes, BoxError>(Bytes::from(bytes)),
                                        (rx, done),
                                    ));
                                }
                                None if done => return None,
                                None => continue,
                            }
                        }
                    }
                }
            });
        Ok(Box::pin(audio_stream))
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
