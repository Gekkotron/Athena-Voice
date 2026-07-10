use std::sync::Arc;

use bytes::Bytes;
use futures::stream::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Tts;

fn is_sentence_boundary(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

pub fn spawn_tts(
    session: SessionId,
    locale: Locale,
    tts: Arc<dyn Tts>,
    mut token_rx: mpsc::Receiver<String>,
    chunk_tx: mpsc::Sender<Bytes>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut seq: u32 = 0;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = token_rx.recv() => match maybe {
                    Some(tok) => {
                        if !buf.is_empty() {
                            buf.push(' ');
                        }
                        buf.push_str(&tok);
                        // Flush on sentence boundary or when buffer is large.
                        let should_flush = buf.chars().last().is_some_and(is_sentence_boundary)
                            || buf.len() >= 100;
                        if should_flush {
                            seq = flush(&tts, session, &locale, &buf, seq, &chunk_tx, &event_tx).await;
                            buf.clear();
                        }
                    }
                    None => {
                        // Drain remaining buffered text as one final sentence.
                        if !buf.is_empty() {
                            let _ = flush(&tts, session, &locale, &buf, seq, &chunk_tx, &event_tx).await;
                        }
                        break;
                    }
                }
            }
        }
    })
}

async fn flush(
    tts: &Arc<dyn Tts>,
    session: SessionId,
    locale: &Locale,
    text: &str,
    mut seq: u32,
    chunk_tx: &mpsc::Sender<Bytes>,
    event_tx: &broadcast::Sender<Event>,
) -> u32 {
    let mut audio = match tts.synthesize(session, locale.clone(), text.to_string()).await {
        Ok(s) => s,
        Err(err) => {
            warn!(error = %err, "tts synthesize failed");
            return seq;
        }
    };
    while let Some(item) = audio.next().await {
        match item {
            Ok(chunk) => {
                let bytes_len = chunk.len();
                if chunk_tx.send(chunk).await.is_err() {
                    return seq;
                }
                let _ = event_tx.send(Event::TtsChunk { session, seq, bytes_len });
                seq = seq.saturating_add(1);
            }
            Err(err) => {
                warn!(error = %err, "tts audio stream error");
                break;
            }
        }
    }
    seq
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use athena_voice_providers::testing::fake_tts::FakeTts;

    #[tokio::test]
    async fn buffers_by_sentence_and_synthesises_each() {
        let (tok_tx, tok_rx) = mpsc::channel(16);
        let (chunk_tx, mut chunk_rx) = mpsc::channel(32);
        let (ev_tx, mut ev_rx) = broadcast::channel(32);
        let tts: Arc<dyn Tts> = Arc::new(FakeTts::new());

        let handle = spawn_tts(
            SessionId::new_v4(),
            Locale::new("fr").unwrap(),
            tts,
            tok_rx,
            chunk_tx,
            ev_tx,
            CancellationToken::new(),
        );

        for t in ["Bonjour.", "Comment", "allez-vous?"] {
            tok_tx.send(t.into()).await.unwrap();
        }
        drop(tok_tx);

        let mut chunks: Vec<Bytes> = Vec::new();
        while let Some(c) = chunk_rx.recv().await {
            chunks.push(c);
        }
        // FakeTts emits one chunk per word. "Bonjour." = 1 chunk, "Comment allez-vous?" = 2 chunks.
        assert_eq!(chunks.len(), 3);
        handle.await.unwrap();

        let mut tts_chunk_events = 0;
        while let Ok(ev) = ev_rx.try_recv() {
            if matches!(ev, Event::TtsChunk { .. }) {
                tts_chunk_events += 1;
            }
        }
        assert_eq!(tts_chunk_events, 3);
    }
}
