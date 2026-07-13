//! STT provider that speaks the generic MQTT request/reply protocol.
//!
//! Wire format: request `{session_id, locale}` on `athena/providers/stt/<name>/request`,
//! streaming responses `{session_id, is_final, text}` on `.../response`.
//! Audio delivery to the provider is out of scope for Plan 3 — a real deployment
//! buffers audio at the satellite adapter and publishes it on a companion topic.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt};
use serde_json::json;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioFrameStream, BoxError, Stt, TranscriptStream};
use athena_voice_core::types::Transcript;

use super::mqtt_client::MqttProviderClient;

pub struct MqttStt {
    name: &'static str,
    client: Arc<MqttProviderClient>,
}

impl MqttStt {
    /// Constructs the provider by connecting to the broker and subscribing to
    /// the response topic for `provider_name`.
    pub async fn connect(
        broker_host: impl Into<String>,
        broker_port: u16,
        provider_name: &'static str,
    ) -> Self {
        let request_topic = format!("athena/providers/stt/{provider_name}/request");
        let response_topic = format!("athena/providers/stt/{provider_name}/response");
        let client = MqttProviderClient::connect(
            broker_host,
            broker_port,
            format!("athena-voice-stt-{provider_name}"),
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
impl Stt for MqttStt {
    async fn transcribe(
        &self,
        session: SessionId,
        locale: Locale,
        _audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError> {
        let request = json!({
            "session_id": session.to_string(),
            "locale": locale.as_str(),
        });
        let payload = Bytes::from(request.to_string().into_bytes());
        let rx = self.client.call_streaming(session, payload).await?;

        // Convert the mpsc<Publish> stream into a Stream<Item = Result<Transcript>>.
        let transcript_stream =
            tokio_stream::wrappers::ReceiverStream::new(rx).filter_map(|publish| async move {
                let v: serde_json::Value = serde_json::from_slice(&publish.payload).ok()?;
                Some(Ok(Transcript {
                    text: v.get("text")?.as_str()?.to_string(),
                    is_final: v.get("is_final")?.as_bool().unwrap_or(false),
                    #[allow(clippy::cast_possible_truncation)]
                    confidence: v
                        .get("confidence")
                        .and_then(serde_json::Value::as_f64)
                        .map(|c| c as f32),
                }))
            });
        // Also stop after a final transcript.
        let ended = transcript_stream.chain(stream::empty());
        Ok(Box::pin(ended))
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
