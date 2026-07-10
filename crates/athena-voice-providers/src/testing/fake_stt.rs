use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{AudioFrameStream, BoxError, Stt, TranscriptStream};
use athena_voice_core::types::Transcript;

pub struct FakeStt {
    preset: HashMap<SessionId, Vec<Transcript>>,
    default: Vec<Transcript>,
}

impl FakeStt {
    #[must_use]
    pub fn builder() -> FakeSttBuilder {
        FakeSttBuilder::default()
    }
}

#[derive(Default)]
pub struct FakeSttBuilder {
    preset: HashMap<SessionId, Vec<Transcript>>,
    default: Vec<Transcript>,
}

impl FakeSttBuilder {
    #[must_use]
    pub fn preset(mut self, session: SessionId, transcripts: Vec<Transcript>) -> Self {
        self.preset.insert(session, transcripts);
        self
    }

    #[must_use]
    pub fn default_transcripts(mut self, transcripts: Vec<Transcript>) -> Self {
        self.default = transcripts;
        self
    }

    #[must_use]
    pub fn build(self) -> FakeStt {
        FakeStt { preset: self.preset, default: self.default }
    }
}

#[async_trait]
impl Stt for FakeStt {
    async fn transcribe(
        &self,
        session: SessionId,
        _locale: Locale,
        _audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError> {
        let items = self
            .preset
            .get(&session)
            .cloned()
            .unwrap_or_else(|| self.default.clone());
        let s = stream::iter(items.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
    }

    fn name(&self) -> &'static str {
        "fake-stt"
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::{self, StreamExt};

    use super::*;

    fn transcript(text: &str, is_final: bool) -> Transcript {
        Transcript { text: text.into(), is_final, confidence: Some(1.0) }
    }

    #[tokio::test]
    async fn emits_preset_transcripts_in_order() {
        let sid = SessionId::new_v4();
        let stt = FakeStt::builder()
            .preset(sid, vec![transcript("bon", false), transcript("bonjour", true)])
            .build();

        let audio: AudioFrameStream = Box::pin(stream::empty());
        let mut ts = stt.transcribe(sid, Locale::new("fr").unwrap(), audio).await.unwrap();
        let a = ts.next().await.unwrap().unwrap();
        let b = ts.next().await.unwrap().unwrap();
        assert!(ts.next().await.is_none());

        assert_eq!(a.text, "bon");
        assert!(!a.is_final);
        assert_eq!(b.text, "bonjour");
        assert!(b.is_final);
    }

    #[tokio::test]
    async fn falls_back_to_default_when_no_preset() {
        let stt = FakeStt::builder()
            .default_transcripts(vec![transcript("hello", true)])
            .build();

        let sid = SessionId::new_v4();
        let audio: AudioFrameStream = Box::pin(stream::empty());
        let mut ts = stt.transcribe(sid, Locale::new("en").unwrap(), audio).await.unwrap();
        let final_t = ts.next().await.unwrap().unwrap();
        assert_eq!(final_t.text, "hello");
        assert!(final_t.is_final);
    }

    #[tokio::test]
    async fn empty_default_yields_empty_stream() {
        let stt = FakeStt::builder().build();
        let sid = SessionId::new_v4();
        let audio: AudioFrameStream = Box::pin(stream::empty());
        let mut ts = stt.transcribe(sid, Locale::new("en").unwrap(), audio).await.unwrap();
        assert!(ts.next().await.is_none());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(FakeStt::builder().build().name(), "fake-stt");
    }
}
