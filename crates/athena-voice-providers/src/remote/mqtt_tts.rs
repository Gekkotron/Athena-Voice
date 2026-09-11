//! TTS provider that speaks the generic MQTT request/reply protocol.
//!
//! Wire format: request `{session_id, locale, text}` on `athena/providers/tts/<name>/request`.
//! Responses on `.../response` carry base64-encoded audio chunks as
//! `{session_id, chunk_b64, done}`; the first message also declares the
//! real format via `{format, sample_rate, channels}` (absent = s16le at
//! 22050 Hz, what pre-metadata workers emitted). The JSON envelope keeps
//! things simple; a future revision can switch to a companion binary
//! topic once bandwidth matters.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde_json::json;

use athena_voice_core::event::AudioFormat;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, Tts, TtsAudio};

use super::mqtt_client::MqttProviderClient;

pub struct MqttTts {
    name: &'static str,
    client: Arc<MqttProviderClient>,
}

impl MqttTts {
    pub async fn connect(
        broker_host: impl Into<String>,
        broker_port: u16,
        topic_root: &str,
        provider_name: &'static str,
    ) -> Self {
        let request_topic = format!("{topic_root}/providers/tts/{provider_name}/request");
        let response_topic = format!("{topic_root}/providers/tts/{provider_name}/response");
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

/// Reads the worker's declared audio format from its first response
/// message. Workers predating the `format` / `sample_rate` fields are
/// read as s16le/22050 — what the bundled `say` worker always emitted.
fn parse_format(first: Option<&serde_json::Value>) -> (AudioFormat, u32) {
    let format = first
        .and_then(|v| v.get("format")?.as_str())
        .map_or(AudioFormat::S16le, |f| match f {
            "opus" => AudioFormat::Opus,
            "f32le" => AudioFormat::F32le,
            "text" => AudioFormat::Text,
            _ => AudioFormat::S16le,
        });
    let sample_rate = first
        .and_then(|v| v.get("sample_rate")?.as_u64())
        .and_then(|r| u32::try_from(r).ok())
        .filter(|r| *r > 0)
        .unwrap_or(22_050);
    (format, sample_rate)
}

#[async_trait]
impl Tts for MqttTts {
    async fn synthesize(
        &self,
        session: SessionId,
        locale: Locale,
        text: String,
    ) -> Result<TtsAudio, BoxError> {
        let request = json!({
            "session_id": session.to_string(),
            "locale": locale.as_str(),
            "text": text,
        });
        let payload = Bytes::from(request.to_string().into_bytes());
        let mut rx = self.client.call_streaming(session, payload).await?;
        let timeout = self.client.request_timeout();

        // The worker's FIRST response message declares the audio format
        // via optional `format` / `sample_rate` fields; absent fields
        // default to s16le/22050 for compatibility with older workers.
        // Await it here so the returned metadata reflects reality.
        let first = loop {
            match tokio::time::timeout(timeout, rx.recv()).await {
                Err(_) | Ok(None) => break None,
                Ok(Some(publish)) => {
                    match serde_json::from_slice::<serde_json::Value>(&publish.payload) {
                        Ok(v) => break Some(v),
                        Err(_) => continue,
                    }
                }
            }
        };
        let (format, sample_rate) = parse_format(first.as_ref());

        // The stream must terminate: on the worker's `done: true` marker, on
        // a per-message timeout (worker died mid-stream), or on channel
        // close. A `done` message may itself carry a final chunk. The first
        // message (already consumed above) is replayed into the stream.
        let audio_stream = futures::stream::unfold(
            (rx, first, false),
            move |(mut rx, pending, finished)| async move {
                if finished {
                    return None;
                }
                let mut next_pending = pending;
                loop {
                    let v = match next_pending.take() {
                        Some(v) => v,
                        None => match tokio::time::timeout(timeout, rx.recv()).await {
                            Err(_) | Ok(None) => return None,
                            Ok(Some(publish)) => {
                                match serde_json::from_slice::<serde_json::Value>(&publish.payload)
                                {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                }
                            }
                        },
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
                                (rx, None, done),
                            ));
                        }
                        None if done => return None,
                        None => continue,
                    }
                }
            },
        );
        Ok(TtsAudio {
            format,
            sample_rate,
            stream: Box::pin(audio_stream),
        })
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn declared_format_is_honoured() {
        let msg = json!({"format": "s16le", "sample_rate": 16_000, "chunk_b64": ""});
        assert_eq!(parse_format(Some(&msg)), (AudioFormat::S16le, 16_000));
        let msg = json!({"format": "opus", "sample_rate": 48_000});
        assert_eq!(parse_format(Some(&msg)), (AudioFormat::Opus, 48_000));
    }

    #[test]
    fn legacy_workers_default_to_s16le_22050() {
        // No format fields at all (pre-metadata workers), and a worker
        // that died before answering.
        let msg = json!({"chunk_b64": "", "done": false});
        assert_eq!(parse_format(Some(&msg)), (AudioFormat::S16le, 22_050));
        assert_eq!(parse_format(None), (AudioFormat::S16le, 22_050));
    }

    #[test]
    fn nonsense_format_values_fall_back_rather_than_panic() {
        let msg = json!({"format": "flac", "sample_rate": 0});
        assert_eq!(parse_format(Some(&msg)), (AudioFormat::S16le, 22_050));
    }
}
