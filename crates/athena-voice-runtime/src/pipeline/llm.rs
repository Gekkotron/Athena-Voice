use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Llm;

/// Spoken when the LLM backend fails before producing anything — silence is
/// the one thing a voice assistant must never answer with.
fn apology(locale: &Locale) -> &'static str {
    if locale.as_str().starts_with("fr") {
        "désolé, je ne peux pas répondre pour le moment."
    } else {
        "sorry, I can't answer right now."
    }
}

pub fn spawn_llm(
    session: SessionId,
    locale: Locale,
    llm: Arc<dyn Llm>,
    mut prompt_rx: mpsc::Receiver<String>,
    token_tx: mpsc::Sender<String>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = prompt_rx.recv() => match maybe {
                    Some(prompt) => {
                        let mut tokens = match llm
                            .complete(session, locale.clone(), prompt, vec![])
                            .await
                        {
                            Ok(s) => s,
                            Err(err) => {
                                warn!(error = %err, "llm complete returned error");
                                if token_tx.send(apology(&locale).to_string()).await.is_err() {
                                    return;
                                }
                                continue;
                            }
                        };
                        let mut sent_any = false;
                        while let Some(item) = tokens.next().await {
                            match item {
                                Ok(t) => {
                                    sent_any = true;
                                    if token_tx.send(t).await.is_err() {
                                        return;
                                    }
                                }
                                Err(err) => {
                                    warn!(error = %err, "llm token stream error");
                                    // Failed before saying anything: apologise
                                    // rather than leaving the user in silence.
                                    if !sent_any
                                        && token_tx
                                            .send(apology(&locale).to_string())
                                            .await
                                            .is_err()
                                    {
                                        return;
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use athena_voice_providers::testing::fake_llm::FakeLlm;

    #[tokio::test]
    async fn failing_backend_speaks_an_apology() {
        struct BrokenLlm;
        #[async_trait::async_trait]
        impl Llm for BrokenLlm {
            async fn complete(
                &self,
                _s: SessionId,
                _l: Locale,
                _p: String,
                _h: Vec<(String, String)>,
            ) -> Result<athena_voice_core::provider::CompletionStream, athena_voice_core::provider::BoxError>
            {
                Err("connection refused".into())
            }
            fn name(&self) -> &'static str {
                "broken"
            }
        }

        let (p_tx, p_rx) = mpsc::channel(4);
        let (tok_tx, mut tok_rx) = mpsc::channel(16);
        spawn_llm(
            SessionId::new_v4(),
            Locale::new("fr").unwrap(),
            Arc::new(BrokenLlm),
            p_rx,
            tok_tx,
            CancellationToken::new(),
        );

        p_tx.send("raconte-moi une blague".into()).await.unwrap();
        let spoken = tok_rx.recv().await.expect("apology token");
        assert!(spoken.contains("désolé"), "got: {spoken}");
    }

    #[tokio::test]
    async fn streams_tokens_for_prompt() {
        let (p_tx, p_rx) = mpsc::channel(4);
        let (tok_tx, mut tok_rx) = mpsc::channel(16);
        let llm: Arc<dyn Llm> =
            Arc::new(FakeLlm::builder().rule("weather", "il fait beau").build());
        let handle = spawn_llm(
            SessionId::new_v4(),
            Locale::new("fr").unwrap(),
            llm,
            p_rx,
            tok_tx,
            CancellationToken::new(),
        );

        p_tx.send("quel est le weather".into()).await.unwrap();
        drop(p_tx);

        let mut collected = Vec::new();
        while let Some(t) = tok_rx.recv().await {
            collected.push(t);
        }
        assert_eq!(collected, vec!["il", "fait", "beau"]);
        handle.await.unwrap();
    }
}
