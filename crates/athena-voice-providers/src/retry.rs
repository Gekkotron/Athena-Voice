use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{
    AudioFrameStream, AudioStream, BoxError, CompletionStream, Llm, Stt, TranscriptStream, Tts,
};

use crate::circuit::CircuitBreaker;

/// Retry policy for a single provider call.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub backoff: Vec<Duration>,
}

impl RetryConfig {
    /// LLM never retries — the model is nondeterministic and a re-roll produces
    /// a different answer.
    #[must_use]
    pub fn llm() -> Self {
        Self {
            max_attempts: 1,
            backoff: vec![],
        }
    }

    /// STT: the audio stream can't be replayed without buffering, so retry is
    /// effectively single-attempt at this layer. Real retry-on-audio should
    /// happen at a level that owns the raw PCM.
    #[must_use]
    pub fn stt() -> Self {
        Self {
            max_attempts: 1,
            backoff: vec![],
        }
    }

    /// TTS: text is owned and cheap to clone; retry twice on transient failure.
    #[must_use]
    pub fn tts() -> Self {
        Self {
            max_attempts: 2,
            backoff: vec![Duration::from_millis(200)],
        }
    }
}

// -----------------------------------------------------------------------------
//  Stt
// -----------------------------------------------------------------------------

pub struct RetryingStt {
    inner: Arc<dyn Stt>,
    circuit: Arc<CircuitBreaker>,
}

impl RetryingStt {
    #[must_use]
    pub fn new(inner: Arc<dyn Stt>, circuit: Arc<CircuitBreaker>) -> Self {
        Self { inner, circuit }
    }
}

#[async_trait]
impl Stt for RetryingStt {
    async fn transcribe(
        &self,
        session: SessionId,
        locale: Locale,
        audio: AudioFrameStream,
    ) -> Result<TranscriptStream, BoxError> {
        if self.circuit.can_call().is_err() {
            return Err("stt circuit open".into());
        }
        match self.inner.transcribe(session, locale, audio).await {
            Ok(s) => {
                self.circuit.record_success();
                Ok(s)
            }
            Err(e) => {
                self.circuit.record_failure();
                Err(e)
            }
        }
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

// -----------------------------------------------------------------------------
//  Llm
// -----------------------------------------------------------------------------

pub struct RetryingLlm {
    inner: Arc<dyn Llm>,
    circuit: Arc<CircuitBreaker>,
}

impl RetryingLlm {
    #[must_use]
    pub fn new(inner: Arc<dyn Llm>, circuit: Arc<CircuitBreaker>) -> Self {
        Self { inner, circuit }
    }
}

#[async_trait]
impl Llm for RetryingLlm {
    async fn complete(
        &self,
        session: SessionId,
        locale: Locale,
        prompt: String,
        history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        if self.circuit.can_call().is_err() {
            return Err("llm circuit open".into());
        }
        match self.inner.complete(session, locale, prompt, history).await {
            Ok(s) => {
                self.circuit.record_success();
                Ok(s)
            }
            Err(e) => {
                self.circuit.record_failure();
                Err(e)
            }
        }
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

// -----------------------------------------------------------------------------
//  Tts (does real retry with backoff)
// -----------------------------------------------------------------------------

pub struct RetryingTts {
    inner: Arc<dyn Tts>,
    circuit: Arc<CircuitBreaker>,
    config: RetryConfig,
}

impl RetryingTts {
    #[must_use]
    pub fn new(inner: Arc<dyn Tts>, circuit: Arc<CircuitBreaker>, config: RetryConfig) -> Self {
        Self {
            inner,
            circuit,
            config,
        }
    }
}

#[async_trait]
impl Tts for RetryingTts {
    async fn synthesize(
        &self,
        session: SessionId,
        locale: Locale,
        text: String,
    ) -> Result<AudioStream, BoxError> {
        if self.circuit.can_call().is_err() {
            return Err("tts circuit open".into());
        }
        let mut last_err: Option<BoxError> = None;
        for attempt in 0..self.config.max_attempts {
            match self
                .inner
                .synthesize(session, locale.clone(), text.clone())
                .await
            {
                Ok(s) => {
                    self.circuit.record_success();
                    return Ok(s);
                }
                Err(e) => {
                    self.circuit.record_failure();
                    last_err = Some(e);
                    if attempt + 1 < self.config.max_attempts
                        && let Some(&delay) = self.config.backoff.get(attempt as usize)
                    {
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| "tts retry exhausted".into()))
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use async_trait::async_trait;
    use futures::stream;

    use super::*;

    struct FailingThenSuccessTts {
        remaining_failures: AtomicU32,
    }

    #[async_trait]
    impl Tts for FailingThenSuccessTts {
        async fn synthesize(
            &self,
            _session: SessionId,
            _locale: Locale,
            _text: String,
        ) -> Result<AudioStream, BoxError> {
            let left = self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            if left > 0 {
                return Err("boom".into());
            }
            Ok(Box::pin(stream::empty()))
        }

        fn name(&self) -> &'static str {
            "failing-tts"
        }
    }

    #[tokio::test]
    async fn retrying_tts_recovers_within_max_attempts() {
        let inner: Arc<dyn Tts> = Arc::new(FailingThenSuccessTts {
            remaining_failures: AtomicU32::new(1),
        });
        let circuit = Arc::new(CircuitBreaker::new(
            10,
            Duration::from_secs(60),
            Duration::from_secs(15),
        ));
        let retrying = RetryingTts::new(inner, circuit, RetryConfig::tts());
        let res = retrying
            .synthesize(SessionId::new_v4(), Locale::new("fr").unwrap(), "hi".into())
            .await;
        assert!(res.is_ok(), "expected recovery within retries");
    }

    #[tokio::test]
    async fn retrying_tts_gives_up_after_max_attempts() {
        let inner: Arc<dyn Tts> = Arc::new(FailingThenSuccessTts {
            remaining_failures: AtomicU32::new(10),
        });
        let circuit = Arc::new(CircuitBreaker::new(
            100,
            Duration::from_secs(60),
            Duration::from_secs(15),
        ));
        let retrying = RetryingTts::new(inner, circuit, RetryConfig::tts());
        let res = retrying
            .synthesize(SessionId::new_v4(), Locale::new("fr").unwrap(), "hi".into())
            .await;
        assert!(res.is_err(), "expected exhaustion");
    }
}
