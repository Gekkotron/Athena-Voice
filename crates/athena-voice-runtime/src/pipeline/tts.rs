use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Tts;

use crate::pipeline::sentence::{IDLE_FLUSH, SentenceBuffer};
use crate::pipeline::sink::SinkMsg;

/// What the router and the LLM feed the TTS stage. `AnswerEnd` must travel
/// in-band with the tokens: it means "everything before me is the whole
/// answer", which is only true if it cannot overtake the last token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TtsMsg {
    Token(String),
    AnswerEnd,
}

impl From<&str> for TtsMsg {
    fn from(s: &str) -> Self {
        Self::Token(s.to_string())
    }
}

impl From<String> for TtsMsg {
    fn from(s: String) -> Self {
        Self::Token(s)
    }
}

pub fn spawn_tts(
    session: SessionId,
    locale: Locale,
    tts: Arc<dyn Tts>,
    mut token_rx: mpsc::Receiver<TtsMsg>,
    chunk_tx: mpsc::Sender<SinkMsg>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let mut barge_rx = event_tx.subscribe();
    tokio::spawn(async move {
        let mut buf = SentenceBuffer::new();
        let mut seq: u32 = 0;
        // Some(deadline) while `buf` holds an unpunctuated fragment waiting
        // for the idle flush. Anchored to a fixed instant — a `sleep`
        // rebuilt from `IDLE_FLUSH` on every `select!` iteration would never
        // elapse under any unrelated event-bus traffic, since every
        // session's events wake `barge_rx.recv()` and re-enter the loop.
        let mut flush_deadline: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                ev = barge_rx.recv() => {
                    if is_barge_in_for(&ev, session) {
                        // Drop queued text: the previous response is dead.
                        buf.clear();
                        flush_deadline = None;
                    }
                    // Ignore lag / other events — a lagged BargeIn is a corner
                    // case that only fires under extreme event-bus pressure.
                }
                () = async {
                    match flush_deadline {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    flush_deadline = None;
                    if let Some(sentence) = buf.take() {
                        seq = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                    }
                }
                maybe = token_rx.recv() => {
                    let Some(msg) = maybe else {
                        if let Some(sentence) = buf.take() {
                            let _ = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                        }
                        break;
                    };
                    let tok = match msg {
                        TtsMsg::Token(t) => t,
                        // Speak whatever is still buffered — the answer may
                        // have ended on an unpunctuated fragment that the
                        // idle flush hasn't reached yet — then tell the sink.
                        TtsMsg::AnswerEnd => {
                            flush_deadline = None;
                            if let Some(sentence) = buf.take() {
                                seq = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                            }
                            if chunk_tx.send(SinkMsg::AnswerEnd).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    match buf.push(&tok) {
                        Some(sentence) => {
                            flush_deadline = None;
                            seq = flush(&tts, session, &locale, &sentence, seq, &chunk_tx, &event_tx, &mut barge_rx).await;
                        }
                        None => {
                            flush_deadline = Some(tokio::time::Instant::now() + IDLE_FLUSH);
                        }
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
    chunk_tx: &mpsc::Sender<SinkMsg>,
    event_tx: &broadcast::Sender<Event>,
    barge_rx: &mut broadcast::Receiver<Event>,
) -> u32 {
    // Announce what is about to be spoken — satellites use this to show the
    // answer as text.
    let _ = event_tx.send(Event::TtsText {
        session,
        text: text.to_string(),
    });
    let audio = match tts
        .synthesize(session, locale.clone(), text.to_string())
        .await
    {
        Ok(s) => s,
        Err(err) => {
            warn!(error = %err, "tts synthesize failed");
            return seq;
        }
    };
    // Declare the provider's real format before its chunks; the sink
    // publishes it once as tts/meta.
    if chunk_tx
        .send(SinkMsg::Format {
            format: audio.format,
            sample_rate: audio.sample_rate,
        })
        .await
        .is_err()
    {
        return seq;
    }
    let mut audio = audio.stream;
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
                        if chunk_tx.send(SinkMsg::Chunk(chunk)).await.is_err() {
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
    use athena_voice_core::event::{AudioFormat, BargeInReason};
    use athena_voice_providers::testing::fake_tts::FakeTts;

    fn chunk_bytes(msg: SinkMsg) -> Option<bytes::Bytes> {
        match msg {
            SinkMsg::Chunk(b) => Some(b),
            SinkMsg::Format { .. } | SinkMsg::AnswerEnd => None,
        }
    }

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
        for tok in ["je ", "ne ", "sais ", "pas"] {
            tok_tx.send(TtsMsg::Token(tok.to_string())).await.unwrap();
        }
        // The provider's format is declared before its first chunk.
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), chunk_rx.recv())
            .await
            .expect("idle flush must synthesize buffered text")
            .expect("message");
        assert!(
            matches!(
                first,
                SinkMsg::Format {
                    format: AudioFormat::Text,
                    ..
                }
            ),
            "expected a Format message first, got {first:?}"
        );
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), chunk_rx.recv())
            .await
            .expect("chunk after format")
            .and_then(chunk_bytes)
            .expect("chunk");
        assert_eq!(&chunk[..], b"je");
        drop(tok_tx);
    }

    /// The satellite re-arms on `done`, which the sink publishes when it sees
    /// `AnswerEnd`. So an answer that ends on an unpunctuated fragment must be
    /// spoken *before* the marker is forwarded, and the marker must arrive
    /// without waiting for the idle flush or for the channel to close — that
    /// wait is exactly what left devices deaf for their whole reply timeout.
    #[tokio::test]
    async fn answer_end_flushes_the_tail_then_signals_the_sink() {
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

        // No terminal punctuation, and the channel stays open afterwards.
        tok_tx.send(TtsMsg::Token("il fait beau".into())).await.unwrap();
        tok_tx.send(TtsMsg::AnswerEnd).await.unwrap();

        let mut spoken = 0usize;
        let mut ended = false;
        // IDLE_FLUSH is 800ms; anything under that proves we did not merely
        // wait for the idle timer to rescue us.
        let deadline = std::time::Duration::from_millis(400);
        while let Ok(Some(msg)) = tokio::time::timeout(deadline, chunk_rx.recv()).await {
            match msg {
                SinkMsg::Chunk(_) => spoken += 1,
                SinkMsg::Format { .. } => {}
                SinkMsg::AnswerEnd => {
                    ended = true;
                    break;
                }
            }
        }
        assert!(spoken > 0, "the unpunctuated tail was never spoken");
        assert!(ended, "AnswerEnd never reached the sink");
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

        for t in ["Bonjour.", "Comment ", "allez-vous?"] {
            tok_tx.send(t.into()).await.unwrap();
        }
        drop(tok_tx);

        let mut chunks: Vec<bytes::Bytes> = Vec::new();
        while let Some(c) = chunk_rx.recv().await {
            if let Some(b) = chunk_bytes(c) {
                chunks.push(b);
            }
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

        let mut got: Vec<bytes::Bytes> = Vec::new();
        while let Some(c) = chunk_rx.recv().await {
            if let Some(b) = chunk_bytes(c) {
                got.push(b);
            }
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
