use async_trait::async_trait;
use futures::stream::{self, StreamExt};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, CompletionStream, Llm};

pub struct FakeLlm {
    rules: Vec<(String, String)>,
}

impl FakeLlm {
    #[must_use]
    pub fn builder() -> FakeLlmBuilder {
        FakeLlmBuilder::default()
    }
}

#[derive(Default)]
pub struct FakeLlmBuilder {
    rules: Vec<(String, String)>,
}

impl FakeLlmBuilder {
    #[must_use]
    pub fn rule(mut self, prompt_substr: impl Into<String>, response: impl Into<String>) -> Self {
        self.rules.push((prompt_substr.into(), response.into()));
        self
    }

    #[must_use]
    pub fn build(self) -> FakeLlm {
        FakeLlm { rules: self.rules }
    }
}

fn fallback(locale: &Locale) -> &'static str {
    if locale.as_str().starts_with("fr") {
        "je ne sais pas"
    } else {
        "i don't know"
    }
}

#[async_trait]
impl Llm for FakeLlm {
    async fn complete(
        &self,
        _session: SessionId,
        locale: Locale,
        prompt: String,
        _history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        let response = self
            .rules
            .iter()
            .find(|(sub, _)| prompt.contains(sub))
            .map_or_else(|| fallback(&locale).to_string(), |(_, r)| r.clone());
        let tokens: Vec<String> = response.split_whitespace().map(String::from).collect();
        let s = stream::iter(tokens.into_iter().map(Ok::<_, BoxError>));
        Ok(Box::pin(s.boxed()))
    }

    fn name(&self) -> &'static str {
        "fake-llm"
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn matched_rule_streams_tokens() {
        let llm = FakeLlm::builder().rule("weather", "il fait beau").build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("fr").unwrap(),
                "quel est le weather".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(t) = tokens.next().await {
            out.push(t.unwrap());
        }
        assert_eq!(out, vec!["il", "fait", "beau"]);
    }

    #[tokio::test]
    async fn unmatched_fr_returns_je_ne_sais_pas() {
        let llm = FakeLlm::builder().build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("fr").unwrap(),
                "quel est le sens de la vie".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut joined = String::new();
        while let Some(t) = tokens.next().await {
            joined.push_str(&t.unwrap());
            joined.push(' ');
        }
        assert!(joined.contains("je ne sais pas"));
    }

    #[tokio::test]
    async fn unmatched_en_returns_i_dont_know() {
        let llm = FakeLlm::builder().build();
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "meaning of life".into(),
                vec![],
            )
            .await
            .unwrap();
        let mut joined = String::new();
        while let Some(t) = tokens.next().await {
            joined.push_str(&t.unwrap());
            joined.push(' ');
        }
        assert!(joined.contains("don't know"));
    }
}
