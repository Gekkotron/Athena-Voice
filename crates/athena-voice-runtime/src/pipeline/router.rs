use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::event::Event;
use athena_voice_core::ids::SessionId;
use athena_voice_core::types::Transcript;

/// Plan 2: no skills yet, so every final transcript falls through to LLM.
pub fn spawn_router(
    mut rx: mpsc::Receiver<Transcript>,
    llm_tx: mpsc::Sender<String>,
    event_tx: broadcast::Sender<Event>,
    session: SessionId,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(t) if t.is_final => {
                        let _ = event_tx.send(Event::LlmFallback { session });
                        if llm_tx.send(t.text).await.is_err() {
                            break;
                        }
                    }
                    Some(_) => {} // partials dropped in Plan 2
                    None => break,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_finals_reach_llm_and_emit_fallback() {
        let (t_tx, t_rx) = mpsc::channel(4);
        let (llm_tx, mut llm_rx) = mpsc::channel(4);
        let (ev_tx, mut ev_rx) = broadcast::channel(16);
        let sid = SessionId::new_v4();
        let handle = spawn_router(t_rx, llm_tx, ev_tx, sid, CancellationToken::new());

        t_tx.send(Transcript { text: "bon".into(), is_final: false, confidence: None })
            .await
            .unwrap();
        t_tx.send(Transcript { text: "bonjour".into(), is_final: true, confidence: None })
            .await
            .unwrap();
        drop(t_tx);

        let prompt = llm_rx.recv().await.unwrap();
        assert_eq!(prompt, "bonjour");
        assert!(llm_rx.recv().await.is_none());
        handle.await.unwrap();

        let mut got_fallback = false;
        while let Ok(ev) = ev_rx.try_recv() {
            if matches!(ev, Event::LlmFallback { .. }) {
                got_fallback = true;
            }
        }
        assert!(got_fallback);
    }
}
