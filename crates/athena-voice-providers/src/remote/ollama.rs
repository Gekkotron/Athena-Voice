use std::time::Duration;

use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::{BoxError, CompletionStream, Llm};

/// LLM provider backed by an Ollama HTTP endpoint (`/api/chat`).
///
/// Streams tokens back as they arrive from Ollama's line-delimited JSON response.
pub struct OllamaLlm {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaLlm {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client,
        }
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

#[derive(Deserialize)]
struct ChatChunk {
    message: ChatChunkMessage,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize)]
struct ChatChunkMessage {
    #[serde(default)]
    content: String,
}

fn system_prompt(locale: &Locale) -> &'static str {
    if locale.as_str().starts_with("fr") {
        "Tu es un assistant vocal utile, concis et amical. Réponds toujours en français."
    } else {
        "You are a helpful, concise, friendly voice assistant. Always reply in English."
    }
}

#[async_trait]
impl Llm for OllamaLlm {
    async fn complete(
        &self,
        _session: SessionId,
        locale: Locale,
        prompt: String,
        _history: Vec<(String, String)>,
    ) -> Result<CompletionStream, BoxError> {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let sys = system_prompt(&locale);
        let body = ChatRequest {
            model: &self.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: sys,
                },
                ChatMessage {
                    role: "user",
                    content: &prompt,
                },
            ],
            stream: true,
        };
        let response = self.client.post(&url).json(&body).send().await?;
        if !response.status().is_success() {
            return Err(format!("ollama http {}", response.status()).into());
        }
        // Ollama streams line-delimited JSON objects. Convert the bytes stream
        // into a token stream by decoding whole lines.
        let byte_stream = response.bytes_stream();
        let token_stream = decode_ndjson_tokens(byte_stream);
        Ok(Box::pin(token_stream))
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}

fn decode_ndjson_tokens<S>(byte_stream: S) -> impl futures::Stream<Item = Result<String, BoxError>>
where
    S: futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send,
{
    async_stream::try_stream! {
        futures::pin_mut!(byte_stream);
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buf.extend_from_slice(&chunk);
            // Emit each complete line.
            while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let line = &line[..line.len().saturating_sub(1)];  // strip trailing \n
                if line.is_empty() {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_slice::<ChatChunk>(line) {
                    if !parsed.message.content.is_empty() {
                        yield parsed.message.content;
                    }
                    if parsed.done {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::stream::StreamExt;

    use super::*;

    #[tokio::test]
    async fn happy_path_streams_tokens() {
        let mut server = mockito::Server::new_async().await;
        let mock_body = "\
{\"message\":{\"content\":\"hello\"},\"done\":false}\n\
{\"message\":{\"content\":\" world\"},\"done\":true}\n";
        let _m = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_header("content-type", "application/x-ndjson")
            .with_body(mock_body)
            .create_async()
            .await;

        let llm = OllamaLlm::new(server.url(), "test");
        let mut tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "hi".into(),
                vec![],
            )
            .await
            .expect("complete");

        let mut all = Vec::new();
        while let Some(t) = tokens.next().await {
            all.push(t.unwrap());
        }
        assert_eq!(all.concat(), "hello world");
    }

    #[tokio::test]
    async fn connection_refused_returns_err_not_hang() {
        // Port 9 (discard) refuses connections on macOS/Linux dev machines.
        let llm = OllamaLlm::new("http://127.0.0.1:9", "test");
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            llm.complete(
                SessionId::new_v4(),
                Locale::new("fr").unwrap(),
                "salut".into(),
                vec![],
            ),
        )
        .await
        .expect("must fail fast, not hang");
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn malformed_lines_are_skipped_and_stream_ends_without_done() {
        let mut server = mockito::Server::new_async().await;
        // Garbage line + valid token, then the stream just ends (no done:true
        // marker) — the token stream must still terminate.
        let mock_body = "\
not json at all\n\
{\"message\":{\"content\":\"ok\"},\"done\":false}\n";
        let _m = server
            .mock("POST", "/api/chat")
            .with_status(200)
            .with_body(mock_body)
            .create_async()
            .await;

        let llm = OllamaLlm::new(server.url(), "test");
        let tokens = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "hi".into(),
                vec![],
            )
            .await
            .expect("complete");

        let all: Vec<_> = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokens.collect::<Vec<_>>(),
        )
        .await
        .expect("stream must terminate without done marker");
        let texts: Vec<String> = all.into_iter().map(Result::unwrap).collect();
        assert_eq!(texts, vec!["ok".to_string()]);
    }

    #[tokio::test]
    async fn http_error_returns_err() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/api/chat")
            .with_status(500)
            .with_body("boom")
            .create_async()
            .await;

        let llm = OllamaLlm::new(server.url(), "test");
        let res = llm
            .complete(
                SessionId::new_v4(),
                Locale::new("en").unwrap(),
                "hi".into(),
                vec![],
            )
            .await;
        assert!(res.is_err());
    }
}
