use std::error::Error as StdError;
use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;

use crate::ids::{Locale, SessionId};
use crate::types::{AudioFrame, Transcript};

pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

pub type AudioFrameStream = Pin<Box<dyn Stream<Item = AudioFrame> + Send>>;
pub type TranscriptStream = Pin<Box<dyn Stream<Item = Result<Transcript, BoxError>> + Send>>;
pub type CompletionStream = Pin<Box<dyn Stream<Item = Result<String, BoxError>> + Send>>;
pub type AudioStream = Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>;

#[async_trait]
pub trait Stt: Send + Sync {
    async fn transcribe(
        &self,
        session: SessionId,
        locale: Locale,
        audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError>;

    fn name(&self) -> &'static str;
}

#[async_trait]
pub trait Llm: Send + Sync {
    async fn complete(
        &self,
        session: SessionId,
        locale: Locale,
        prompt: String,
        history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError>;

    fn name(&self) -> &'static str;
}

#[async_trait]
pub trait Tts: Send + Sync {
    async fn synthesize(
        &self,
        session: SessionId,
        locale: Locale,
        text: String,
    ) -> Result<AudioStream, BoxError>;

    fn name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_dyn_stt(_: &(dyn Stt)) {}
    fn _assert_dyn_llm(_: &(dyn Llm)) {}
    fn _assert_dyn_tts(_: &(dyn Tts)) {}

    #[test]
    fn traits_are_object_safe() {
        // `_assert_*` fns above only compile if the traits are object-safe.
        // A passing `cargo build` implies success; this test documents intent.
    }
}
