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

/// Buffered text with no sentence boundary is flushed after this much
/// token-channel silence — LLM answers don't reliably end in punctuation
/// ("je ne sais pas"), and without this they would never be spoken.
const IDLE_FLUSH: std::time::Duration = std::time::Duration::from_millis(800);

pub fn spawn_tts(
    session: SessionId,
    locale: Locale,
    tts: Arc<dyn Tts>,
    mut token_rx: mpsc::Receiver<String>,
    chunk_tx: mpsc::Sender<Bytes>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let mut barge_rx = event_tx.subscribe();
    tokio::spawn(async move {
        let mut buf = String::new();
        let mut seq: u32 = 0;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                ev = barge_rx.recv() => {
                    if is_barge_in_for(&ev, session) {
                        // Drop queued text: the previous response is dead.
                        buf.clear();
                    }
                    // Ignore lag / other events — a lagged BargeIn is a corner
                    // case that only fires under extreme event-bus pressure.
                }
                () = tokio::time::sleep(IDLE_FLUSH), if !buf.is_empty() => {
                    // Token channel went quiet mid-sentence: speak what we have.
                    seq = flush(&tts, session, &locale, &buf, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                    buf.clear();
                }
                maybe = token_rx.recv() => {
                    let Some(tok) = maybe else {
                        // Drain remaining buffered text as one final sentence.
                        if !buf.is_empty() {
                            let _ = flush(&tts, session, &locale, &buf, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                        }
                        break;
                    };
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(&tok);
                    // Flush on sentence boundary or when buffer is large.
                    let should_flush = buf.chars().last().is_some_and(is_sentence_boundary)
                        || buf.len() >= 100;
                    if should_flush {
                        seq = flush(&tts, session, &locale, &buf, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                        buf.clear();
                    }
                }
            }
        }
    })
}

fn is_barge_in_for(ev: &Result<Event, broadcast::error::RecvError>, session: SessionId) -> bool {
    matches!(ev, Ok(Event::BargeIn { session: s, .. }) if *s == session)
}

#[allow(clippy::too_many_arguments)]
async fn flush(
    tts: &Arc<dyn Tts>,
    session: SessionId,
    locale: &Locale,
    text: &str,
    mut seq: u32,
    chunk_tx: &mpsc::Sender<Bytes>,
    event_tx: &broadcast::Sender<Event>,
    barge_rx: &mut broadcast::Receiver<Event>,
) -> u32 {
    let mut audio = match tts
        .synthesize(session, locale.clone(), text.to_string())
        .await
    {
        Ok(s) => s,
        Err(err) => {
            warn!(error = %err, "tts synthesize failed");
            return seq;
        }
    };
    loop {
        tokio::select! {
            biased;
            ev = barge_rx.recv() => {
                if is_barge_in_for(&ev, session) {
                    // Abort the in-flight synthesis — the previous response
                    // has been superseded, its audio must not reach the sink.
                    return seq;
                }
            }
            item = audio.next() => {
                let Some(item) = item else { break; };
                match item {
                    Ok(chunk) => {
                        let bytes_len = chunk.len();
                        if chunk_tx.send(chunk).await.is_err() {
                            return seq;
                        }
                        let _ = event_tx.send(Event::TtsChunk {
                            session,
                            seq,
                            bytes_len,
                        });
                        seq = seq.saturating_add(1);
                    }
                    Err(err) => {
                        warn!(error = %err, "tts audio stream error");
                        break;
                    }
                }
            }
        }
    }
    seq
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use athena_voice_core::event::BargeInReason;
    use athena_voice_providers::testing::fake_tts::FakeTts;

    #[tokio::test]
    async fn idle_flush_speaks_unpunctuated_answers() {
        let (tok_tx, tok_rx) = mpsc::channel(16);
        let (chunk_tx, mut chunk_rx) = mpsc::channel(32);
        let (ev_tx, _ev_rx) = broadcast::channel(32);
        let tts: Arc<dyn Tts> = Arc::new(FakeTts::new());

        let _handle = spawn_tts(
            SessionId::new_v4(),
            Locale::new("fr").unwrap(),
            tts,
            tok_rx,
            chunk_tx,
            ev_tx,
            CancellationToken::new(),
        );

        // LLM-style tokens with no sentence boundary; the channel STAYS OPEN
        // (a session outlives its answers), so only the idle flush can
        // trigger synthesis.
        for tok in ["je", "ne", "sais", "pas"] {
            tok_tx.send(tok.to_string()).await.unwrap();
        }
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), chunk_rx.recv())
            .await
            .expect("idle flush must synthesize buffered text")
            .expect("chunk");
        assert_eq!(&chunk[..], b"je");
        drop(tok_tx);
    }

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

    #[tokio::test]
    async fn barge_in_flushes_buffered_text_before_synthesis() {
        // Feed a partial sentence with no boundary, so it stays buffered.
        // A BargeIn event must clear the buffer so a subsequent utterance
        // does not concatenate onto the previous one.
        let (tok_tx, tok_rx) = mpsc::channel(16);
        let (chunk_tx, mut chunk_rx) = mpsc::channel(32);
        let (ev_tx, _ev_rx) = broadcast::channel(32);
        let tts: Arc<dyn Tts> = Arc::new(FakeTts::new());
        let session = SessionId::new_v4();

        let handle = spawn_tts(
            session,
            Locale::new("fr").unwrap(),
            tts,
            tok_rx,
            chunk_tx,
            ev_tx.clone(),
            CancellationToken::new(),
        );

        // Buffered, no boundary — nothing flushed yet.
        tok_tx.send("Bonjour".into()).await.unwrap();
        // Give the actor a moment to buffer it.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Signal barge-in for this session: buffer should be dropped.
        ev_tx
            .send(Event::BargeIn {
                session,
                reason: BargeInReason::NewFinalTranscript,
            })
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Now send a fresh sentence — only the new text should be synthesised,
        // not "Bonjour Nouveau.".
        tok_tx.send("Nouveau.".into()).await.unwrap();
        drop(tok_tx);

        let mut got: Vec<Bytes> = Vec::new();
        while let Some(c) = chunk_rx.recv().await {
            got.push(c);
        }
        handle.await.unwrap();

        // FakeTts emits one chunk per word. If the buffer was flushed we get
        // one chunk ("Nouveau."); otherwise we'd get two ("Bonjour Nouveau.").
        assert_eq!(
            got.len(),
            1,
            "barge-in must drop buffered text; got {} chunks",
            got.len()
        );
    }
}
