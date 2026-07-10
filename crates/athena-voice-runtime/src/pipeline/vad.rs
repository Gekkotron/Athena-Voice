use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_core::types::AudioFrame;

/// Plan 2 VAD is passthrough. Plan 3 upgrades to an energy or Silero-VAD based
/// endpoint detector. The `_endpoint_after_silent_frames` parameter documents
/// where the future threshold will live.
pub fn spawn_vad(
    mut rx: mpsc::Receiver<AudioFrame>,
    tx: mpsc::Sender<AudioFrame>,
    cancel: CancellationToken,
    _endpoint_after_silent_frames: u32,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = rx.recv() => match maybe {
                    Some(frame) => {
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

    #[tokio::test]
    async fn passes_through() {
        let (in_tx, in_rx) = mpsc::channel(4);
        let (out_tx, mut out_rx) = mpsc::channel(4);
        spawn_vad(in_rx, out_tx, CancellationToken::new(), 25);

        in_tx
            .send(AudioFrame {
                session: SessionId::new_v4(),
                seq: 0,
                pcm: Bytes::from_static(&[1, 2]),
            })
            .await
            .unwrap();
        drop(in_tx);
        assert!(out_rx.recv().await.is_some());
        assert!(out_rx.recv().await.is_none());
    }
}
