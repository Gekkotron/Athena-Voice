//! LLM provider for any OpenAI-compatible chat endpoint.
//!
//! Works with hosted APIs and self-hosted servers alike (OpenAI, llama.cpp
//! `--server`, vLLM, LM Studio, Ollama's `/v1` endpoint, …). Config:
//!
//! ```toml
//! llm = { openai_compatible = { base_url = "http://192.168.1.10:11434/v1", model = "llama3.2:1b", api_key_env = "OPENAI_API_KEY" } }
//! ```
//!
//! `base_url` is the prefix ending in `/v1`; the provider POSTs to
//! `<base_url>/chat/completions` with `stream: true` and parses the SSE
//! stream (`data: {json}` lines, terminated by `data: [DONE]`).
//! `api_key_env` names an environment variable holding the bearer token —
//! never put the key itself in the config file. Omit it for local servers
//! that don't authenticate.

use std::time::Duration;

use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::Serialize;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, CompletionStream, Llm};

use super::ollama::system_prompt;

pub struct OpenAiCompatLlm {
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAiCompatLlm {
    /// Builds the provider. `api_key_env`, when set, must name an existing
    /// environment variable — failing fast beats a 401 mid-conversation.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key_env: Option<&str>,
    ) -> Result<Self, String> {
        let api_key = match api_key_env {
            Some(var) => Some(
                std::env::var(var)
                    .map_err(|_| format!("api_key_env is set but ${var} is not defined"))?,
            ),
            None => None,
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Ok(Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key,
            client,
        })
    }
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

/// Extracts the token from one SSE data payload; `None` for keep-alives,
/// role-only deltas, or malformed lines (all safely skippable).
fn delta_content(data: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let content = v
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()?;
    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

#[async_trait]
impl Llm for OpenAiCompatLlm {
    async fn complete(
        &self,
        _session: SessionId,
        locale: Locale,
        prompt: String,
        _history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system_prompt(&locale),
                },
                ChatMessage {
                    role: "user",
                    content: &prompt,
                },
            ],
            stream: true,
        };
        let mut request = self.client.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(format!("openai-compatible http {}", response.status()).into());
        }

        let byte_stream = response.bytes_stream();
        let token_stream = decode_sse_tokens(byte_stream);
        Ok(Box::pin(token_stream))
    }

    fn name(&self) -> &'static str {
        "openai-compatible"
    }
}

/// Decodes an SSE byte stream into content tokens. Terminates on
/// `data: [DONE]`, on stream end, or on transport error.
fn decode_sse_tokens<S>(byte_stream: S) -> impl futures::Stream<Item = Result<String, BoxError>>
where
    S: futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send,
{
    async_stream::try_stream! {
        futures::pin_mut!(byte_stream);
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let line = String::from_utf8_lossy(&line[..line.len().saturating_sub(1)]);
                let line = line.trim();
                let Some(data) = line.strip_prefix("data:") else {
                    continue; // empty keep-alive lines, comments, event: fields
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return;
                }
                if let Some(token) = delta_content(data) {
                    yield token;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::StreamExt;

    use super::*;

    fn sse_body(lines: &[&str]) -> String {
        lines
            .iter()
            .map(|l| format!("data: {l}\n\n"))
            .collect::<String>()
    }

    async fn collect(llm: &OpenAiCompatLlm, locale: &str) -> Result<Vec<String>, BoxError> {
        let tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new(locale).unwrap(),
                "hi".into(),
                vec![],
            )
            .await?;
        let items: Vec<_> = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokens.collect::<Vec<_>>(),
        )
        .await
        .expect("stream must terminate");
        items.into_iter().collect()
    }

    #[tokio::test]
    async fn streams_sse_deltas_until_done() {
        let mut server = mockito::Server::new_async().await;
        let body = sse_body(&[
            r#"{"choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"{"choices":[{"delta":{"content":"Bon"}}]}"#,
            r#"{"choices":[{"delta":{"content":"jour"}}]}"#,
            "[DONE]",
        ]);
        let _m = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "text/event-stream")
            .with_body(body)
            .create_async()
            .await;

        let llm = OpenAiCompatLlm::new(format!("{}/v1", server.url()), "m", None).unwrap();
        let tokens = collect(&llm, "fr").await.unwrap();
        assert_eq!(tokens.concat(), "Bonjour");
    }

    #[tokio::test]
    async fn sends_bearer_token_from_env() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/v1/chat/completions")
            .match_header("authorization", "Bearer sk-test-123")
            .with_status(200)
            .with_body(sse_body(&["[DONE]"]))
            .create_async()
            .await;

        // SAFETY: test-local env var; tests touching it use unique names.
        unsafe { std::env::set_var("ATHENA_TEST_OPENAI_KEY", "sk-test-123") };
        let llm = OpenAiCompatLlm::new(
            format!("{}/v1", server.url()),
            "m",
            Some("ATHENA_TEST_OPENAI_KEY"),
        )
        .unwrap();
        let tokens = collect(&llm, "en").await.unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn missing_api_key_env_fails_fast() {
        let Err(err) = OpenAiCompatLlm::new("http://x/v1", "m", Some("ATHENA_TEST_MISSING_KEY"))
        else {
            panic!("must fail when the env var is missing");
        };
        assert!(err.contains("ATHENA_TEST_MISSING_KEY"));
    }

    #[tokio::test]
    async fn malformed_lines_skipped_and_stream_end_terminates() {
        let mut server = mockito::Server::new_async().await;
        // Garbage line, keep-alive, valid token — and NO [DONE] marker.
        let body = format!(
            ": keep-alive\n\ndata: not json\n\n{}",
            sse_body(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#])
        );
        let _m = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let llm = OpenAiCompatLlm::new(format!("{}/v1", server.url()), "m", None).unwrap();
        let tokens = collect(&llm, "en").await.unwrap();
        assert_eq!(tokens, vec!["ok".to_string()]);
    }

    #[tokio::test]
    async fn http_error_and_connection_refused_fail_fast() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/v1/chat/completions")
            .with_status(500)
            .create_async()
            .await;
        let llm = OpenAiCompatLlm::new(format!("{}/v1", server.url()), "m", None).unwrap();
        assert!(collect(&llm, "en").await.is_err());

        let refused = OpenAiCompatLlm::new("http://127.0.0.1:9/v1", "m", None).unwrap();
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            refused.complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "hi".into(),
                vec![],
            ),
        )
        .await
        .expect("must fail fast");
        assert!(res.is_err());
    }
}
