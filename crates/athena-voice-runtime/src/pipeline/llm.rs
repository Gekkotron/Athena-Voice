use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Llm;

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
                                continue;
                            }
                        };
                        while let Some(item) = tokens.next().await {
                            match item {
                                Ok(t) => {
                                    if token_tx.send(t).await.is_err() {
                                        return;
                                    }
                                }
                                Err(err) => {
                                    warn!(error = %err, "llm token stream error");
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
