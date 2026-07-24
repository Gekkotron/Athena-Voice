//! STT provider that speaks the generic MQTT request/reply protocol.
//!
//! Wire format — everything is JSON, one message per publish.
//!
//! Provider → worker, on `athena/providers/stt/<name>/request`:
//! 1. Session start:
//!    `{ "session_id": "<uuid>", "locale": "fr", "format": "s16le", "sample_rate": 16000 }`
//! 2. Audio, one message per frame batch (raw satellite PCM, base64):
//!    `{ "session_id": "<uuid>", "audio_b64": "..." }`
//! 3. End of audio (the session's input stream closed):
//!    `{ "session_id": "<uuid>", "done": true }`
//!
//! Worker → provider, on `athena/providers/stt/<name>/response`:
//! - Transcripts (any number, partial or final, across the whole session):
//!   `{ "session_id": "<uuid>", "text": "...", "is_final": bool, "confidence": f32? }`
//! - Terminal marker once nothing more will be sent (normally after the
//!   audio `done` arrived and the last transcript went out):
//!   `{ "session_id": "<uuid>", "done": true }`
//!
//! The transcript stream ends on the worker's terminal marker, on channel
//! close, or when no message arrives within the client's request timeout —
//! a session must never hang on a dead worker. NB: unlike TTS, a final
//! transcript does NOT end the stream (a session can carry several
//! utterances); only the terminal marker does.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use futures::stream::StreamExt;
use serde_json::json;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioFrameStream, BoxError, Stt, TranscriptStream};
use athena_voice_core::types::Transcript;

use super::mqtt_client::MqttProviderClient;

/// Format constants declared in the session-start message. The satellite
/// audio path forwards PCM verbatim, so this is the contract satellites and
/// STT workers agree on.
pub const AUDIO_FORMAT: &str = "s16le";
pub const AUDIO_SAMPLE_RATE: u32 = 16_000;

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

/// One parsed worker response message.
#[derive(Debug, PartialEq)]
pub(crate) enum SttResponse {
    Transcript(Transcript),
    Done,
    Malformed,
}

pub(crate) fn parse_stt_response(payload: &[u8]) -> SttResponse {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return SttResponse::Malformed;
    };
    if let Some(text) = v.get("text").and_then(serde_json::Value::as_str) {
        return SttResponse::Transcript(Transcript {
            text: text.to_string(),
            is_final: v
                .get("is_final")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            #[allow(clippy::cast_possible_truncation)]
            confidence: v
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .map(|c| c as f32),
        });
    }
    if v.get("done")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return SttResponse::Done;
    }
    SttResponse::Malformed
}

#[async_trait]
impl Stt for MqttStt {
    async fn transcribe(
        &self,
        session: SessionId,
        locale: Locale,
        mut audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError> {
        let request = json!({
            "session_id": session.to_string(),
            "locale": locale.as_str(),
            "format": AUDIO_FORMAT,
            "sample_rate": AUDIO_SAMPLE_RATE,
        });
        let payload = Bytes::from(request.to_string().into_bytes());
        let rx = self.client.call_streaming(session, payload).await?;
        let timeout = self.client.request_timeout();

        // Pump the session's audio frames to the worker as they arrive, then
        // signal end-of-audio. Runs independently of the response stream so
        // transcripts can flow while audio is still being captured.
        let pump_client = self.client.clone();
        let sid = session.to_string();
        tokio::spawn(async move {
            while let Some(frame) = audio.next().await {
                let msg = json!({
                    "session_id": sid,
                    "audio_b64": STANDARD.encode(&frame.pcm),
                });
                if pump_client
                    .publish_request(Bytes::from(msg.to_string().into_bytes()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            let done = json!({ "session_id": sid, "done": true });
            let _ = pump_client
                .publish_request(Bytes::from(done.to_string().into_bytes()))
                .await;
        });

        let transcript_stream = futures::stream::unfold(rx, move |mut rx| async move {
            loop {
                match tokio::time::timeout(timeout, rx.recv()).await {
                    Err(_) | Ok(None) => return None,
                    Ok(Some(publish)) => match parse_stt_response(&publish.payload) {
                        SttResponse::Transcript(t) => {
                            return Some((Ok::<Transcript, BoxError>(t), rx));
                        }
                        SttResponse::Done => return None,
                        SttResponse::Malformed => continue,
                    },
                }
            }
        });
        Ok(Box::pin(transcript_stream))
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transcript_with_optional_fields() {
        let msg = br#"{"session_id":"x","text":"bonjour","is_final":true,"confidence":0.9}"#;
        match parse_stt_response(msg) {
            SttResponse::Transcript(t) => {
                assert_eq!(t.text, "bonjour");
                assert!(t.is_final);
                assert!(t.confidence.is_some_and(|c| (c - 0.9).abs() < 1e-6));
            }
            other => panic!("expected transcript, got {other:?}"),
        }

        let partial = br#"{"session_id":"x","text":"bon"}"#;
        match parse_stt_response(partial) {
            SttResponse::Transcript(t) => {
                assert!(!t.is_final, "is_final defaults to false");
                assert!(t.confidence.is_none());
            }
            other => panic!("expected transcript, got {other:?}"),
        }
    }

    #[test]
    fn parses_terminal_marker_and_rejects_garbage() {
        assert_eq!(
            parse_stt_response(br#"{"session_id":"x","done":true}"#),
            SttResponse::Done
        );
        assert_eq!(
            parse_stt_response(br#"{"session_id":"x","done":false}"#),
            SttResponse::Malformed
        );
        assert_eq!(parse_stt_response(b"not json"), SttResponse::Malformed);
    }
}
