use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::types::AudioFrame;

pub fn spawn_ingest(
    mut rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<AudioFrame>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(frame) => {
                        if frame.pcm.is_empty() {
                            continue;
                        }
                        if tx.send(frame).await.is_err() {
                            break;
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
    use bytes::Bytes;

    use super::*;
    use athena_voice_core::ids::SessionId;

    fn frame(session: SessionId, seq: u32, pcm: &[u8]) -> AudioFrame {
        AudioFrame {
            session,
            seq,
            pcm: Bytes::copy_from_slice(pcm),
        }
    }

    #[tokio::test]
    async fn passes_through_non_empty_frames() {
        let (in_tx, in_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        let handle = spawn_ingest(in_rx, out_tx, CancellationToken::new());

        let sid = SessionId::new_v4();
        in_tx.send(frame(sid, 0, &[1, 2, 3])).await.unwrap();
        drop(in_tx);
        let received = out_rx.recv().await.unwrap();
        assert_eq!(received.seq, 0);
        assert!(out_rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drops_empty_frames() {
        let (in_tx, in_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        spawn_ingest(in_rx, out_tx, CancellationToken::new());

        let sid = SessionId::new_v4();
        in_tx.send(frame(sid, 0, &[])).await.unwrap();
        in_tx.send(frame(sid, 1, &[1, 2])).await.unwrap();
        drop(in_tx);
        let first = out_rx.recv().await.unwrap();
        assert_eq!(first.seq, 1);
        assert!(out_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn cancellation_terminates() {
        let (_in_tx, in_rx) = mpsc::channel::<AudioFrame>(4);
        let (out_tx, _out_rx) = mpsc::channel(4);
        let token = CancellationToken::new();
        let handle = spawn_ingest(in_rx, out_tx, token.clone());
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("timed out")
            .unwrap();
    }
}
