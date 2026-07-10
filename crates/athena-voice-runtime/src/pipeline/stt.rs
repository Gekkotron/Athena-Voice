use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SessionId};
use athena_voice_core::provider::Stt;
use athena_voice_core::types::{AudioFrame, Transcript};

pub fn spawn_stt(
    session: SessionId,
    locale: Locale,
    stt: Arc<dyn Stt>,
    rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<Transcript>,
    event_tx: broadcast::Sender<Event>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let audio_stream = ReceiverStream::new(rx);
        let boxed: athena_voice_core::provider::AudioFrameStream = Box::pin(audio_stream);
        let mut ts = match stt.transcribe(session, locale.clone(), boxed).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, "stt provider transcribe returned error");
                return;
            }
        };
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = ts.next() => match maybe {
                    Some(Ok(t)) => {
                        let ev = if t.is_final {
                            Event::TranscriptFinal { session, text: t.text.clone() }
                        } else {
                            Event::TranscriptPartial { session, text: t.text.clone() }
                        };
                        let _ = event_tx.send(ev);
                        if tx.send(t).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        warn!(error = %err, "stt stream error");
                        break;
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

    use bytes::Bytes;

    use super::*;
    use athena_voice_providers::testing::fake_stt::FakeStt;

    #[tokio::test]
    async fn emits_transcripts_and_events() {
        let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(4);
        let (t_tx, mut t_rx) = mpsc::channel::<Transcript>(4);
        let (ev_tx, mut ev_rx) = broadcast::channel::<Event>(16);
        let sid = SessionId::new_v4();
        let stt: Arc<dyn athena_voice_core::provider::Stt> = Arc::new(
            FakeStt::builder()
                .preset(
                    sid,
                    vec![
                        Transcript { text: "bon".into(), is_final: false, confidence: None },
                        Transcript { text: "bonjour".into(), is_final: true, confidence: None },
                    ],
                )
                .build(),
        );

        let handle = spawn_stt(
            sid,
            Locale::new("fr").unwrap(),
            stt,
            audio_rx,
            t_tx,
            ev_tx,
            CancellationToken::new(),
        );

        audio_tx
            .send(AudioFrame { session: sid, seq: 0, pcm: Bytes::from_static(&[1]) })
            .await
            .unwrap();
        drop(audio_tx);

        let first_transcript = t_rx.recv().await.unwrap();
        assert_eq!(first_transcript.text, "bon");
        assert!(!first_transcript.is_final);
        let second_transcript = t_rx.recv().await.unwrap();
        assert!(second_transcript.is_final);
        assert!(t_rx.recv().await.is_none());
        handle.await.unwrap();

        let mut got_partial = false;
        let mut got_final = false;
        while let Ok(ev) = ev_rx.try_recv() {
            match ev {
                Event::TranscriptPartial { .. } => got_partial = true,
                Event::TranscriptFinal { .. } => got_final = true,
                _ => {}
            }
        }
        assert!(got_partial);
        assert!(got_final);
    }
}
