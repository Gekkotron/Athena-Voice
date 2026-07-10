use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioStream, BoxError, Tts};

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
    ) -> Result<AudioStream, BoxError> {
        let chunks: Vec<Bytes> = text
            .split_whitespace()
            .map(|w| Bytes::copy_from_slice(w.as_bytes()))
            .collect();
        let s = stream::iter(chunks.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
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
    async fn one_chunk_per_word() {
        let tts = FakeTts::new();
        let mut audio = tts
            .synthesize(SessionId::new_v4(), Locale::new("en").unwrap(), "hello world".into())
            .await
            .unwrap();
        let mut chunks = Vec::new();
        while let Some(c) = audio.next().await {
            chunks.push(c.unwrap());
        }
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref(), b"hello");
        assert_eq!(chunks[1].as_ref(), b"world");
    }

    #[tokio::test]
    async fn empty_text_empty_stream() {
        let tts = FakeTts::new();
        let mut audio = tts
            .synthesize(SessionId::new_v4(), Locale::new("en").unwrap(), String::new())
            .await
            .unwrap();
        assert!(audio.next().await.is_none());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FakeTts::new().name(), "fake-tts");
    }
}
