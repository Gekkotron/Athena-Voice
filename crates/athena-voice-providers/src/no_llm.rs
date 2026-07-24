//! The explicit "no LLM" provider.
//!
//! `llm = "none"` makes the assistant fully deterministic and offline for
//! the language stage: skills handle everything they match, and unmatched
//! questions get a short capabilities answer instead of a model call. No
//! API, no key, no download.

use async_trait::async_trait;
use futures::stream;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, CompletionStream, Llm};

pub struct NoLlm;

fn capabilities(locale: &Locale) -> &'static str {
    if locale.as_str().starts_with("fr") {
        "je ne peux pas répondre à cette question. Je peux donner l'heure, la météo, et lancer des minuteurs."
    } else {
        "I can't answer that question. I can tell the time, give the weather, and set timers."
    }
}

#[async_trait]
impl Llm for NoLlm {
    async fn complete(
        &self,
        _session: SessionId,
        locale: Locale,
        _prompt: String,
        _history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        let sentence = capabilities(&locale).to_string();
        Ok(Box::pin(stream::iter([Ok(sentence)])))
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;

    #[tokio::test]
    async fn answers_with_capabilities_in_session_language() {
        for (locale, needle) in [("fr", "l'heure"), ("en", "the time")] {
            let mut tokens = NoLlm
                .complete(
                    SessionId::new_v4(),
                    Locale::new(locale).unwrap(),
                    "anything".into(),
                    vec![],
                )
                .await
                .unwrap();
            let first = tokens.next().await.unwrap().unwrap();
            assert!(first.contains(needle), "{locale}: {first}");
            assert!(tokens.next().await.is_none(), "single-sentence stream");
        }
    }
}
