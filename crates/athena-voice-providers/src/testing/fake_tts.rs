use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt};

use athena_voice_core::event::AudioFormat;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, Tts, TtsAudio};

#[derive(Default)]
pub struct FakeTts;

impl FakeTts {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tts for FakeTts {
    async fn synthesize(
        &self,
        _session: SessionId,
        _locale: Locale,
        text: String,
    ) -> Result<TtsAudio, BoxError> {
        let chunks: Vec<Bytes> = text
            .split_whitespace()
            .map(|w| Bytes::copy_from_slice(w.as_bytes()))
            .collect();
        let s = stream::iter(chunks.into_iter().map(Ok::<_, BoxError>));
        Ok(TtsAudio {
            format: AudioFormat::Text,
            sample_rate: 0,
            stream: Box::pin(s.boxed()),
        })
    }

    fn name(&self) -> &'static str {
        "fake-tts"
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn one_chunk_per_word_with_text_format() {
        let tts = FakeTts::new();
        let audio = tts
            .synthesize(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "hello world".into(),
            )
            .await
            .unwrap();
        assert_eq!(audio.format, AudioFormat::Text);
        let mut stream = audio.stream;
        let mut chunks = Vec::new();
        while let Some(c) = stream.next().await {
            chunks.push(c.unwrap());
        }
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref(), b"hello");
        assert_eq!(chunks[1].as_ref(), b"world");
    }

    #[tokio::test]
    async fn empty_text_empty_stream() {
        let tts = FakeTts::new();
        let audio = tts
            .synthesize(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                String::new(),
            )
            .await
            .unwrap();
        let mut stream = audio.stream;
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FakeTts::new().name(), "fake-tts");
    }
}
