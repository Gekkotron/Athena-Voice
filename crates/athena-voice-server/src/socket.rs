//! Socket protocol and Unix domain socket server.

use std::convert::TryFrom;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::{Buf, BufMut, BytesMut};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::Framed;

use once_cell::sync::OnceCell;
use serde_json::json;

use crate::metrics::{record_asr, record_hotword, record_tts, record_vad};
use crate::runtime::Runtime;
use crate::vad::{VadDetector, split_voice_segments};

/// Socket header: 0xAE.
const HEADER: u8 = 0xAE;

/// Operation codes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Audio input/output.
    Audio = 0x01,
    /// Transcript/Metrics (JSON).
    Transcript = 0x02,
}

impl TryFrom<u8> for Op {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Audio),
            0x02 => Ok(Self::Transcript),
            _ => Err(anyhow::anyhow!("Invalid operation code: {}", value)),
        }
    }
}

/// Socket protocol codec.
#[derive(Debug, Default)]
pub struct SocketCodec;

impl tokio_util::codec::Decoder for SocketCodec {
    type Item = (Op, BytesMut);
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let header = src[0];
        if header != HEADER {
            return Err(anyhow::anyhow!("Invalid header: {}", header));
        }

        let op = Op::try_from(src[1])?;
        let len = u16::from_be_bytes([src[2], src[3]]) as usize;

        if src.len() < 4 + len {
            return Ok(None);
        }

        src.advance(4);
        let payload = src.split_to(len);
        Ok(Some((op, payload)))
    }
}

impl tokio_util::codec::Encoder<(Op, BytesMut)> for SocketCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: (Op, BytesMut), dst: &mut BytesMut) -> Result<(), Self::Error> {
        let (op, payload) = item;
        dst.put_u8(HEADER);
        dst.put_u8(op as u8);
        dst.put_u16(payload.len() as u16);
        dst.put(payload);
        Ok(())
    }
}

/// Start the audio socket server.
pub async fn start_audio_socket(_runtime: Arc<Runtime>) -> anyhow::Result<()> {
    unimplemented!("Audio socket not yet implemented")
}

/// Start the event socket server.
pub async fn start_event_socket(runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let socket_path = runtime.config().event_socket.clone();
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("Listening on {}", socket_path.display());

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_event_stream(stream, runtime.clone()));
    }

    Ok(())
}

/// Handle an event stream.
async fn handle_event_stream(mut stream: UnixStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    loop {
        let metrics = {
            let metrics = runtime.metrics();
            serde_json::json!({
                "vad": {
                    "invocations": metrics.vad_invocations.load(Ordering::Relaxed),
                    "avg_duration_ns": metrics.vad_duration_ns.load(Ordering::Relaxed) /
                                         metrics.vad_invocations.load(Ordering::Relaxed).max(1),
                },
                "hotword": {
                    "invocations": metrics.hotword_invocations.load(Ordering::Relaxed),
                    "avg_duration_ns": metrics.hotword_duration_ns.load(Ordering::Relaxed) /
                                          metrics.hotword_invocations.load(Ordering::Relaxed).max(1),
                    "detections": metrics.hotword_detections.load(Ordering::Relaxed),
                },
                "asr": {
                    "invocations": metrics.asr_invocations.load(Ordering::Relaxed),
                    "avg_duration_ns": metrics.asr_duration_ns.load(Ordering::Relaxed) /
                                         metrics.asr_invocations.load(Ordering::Relaxed).max(1),
                    "successes": metrics.asr_successes.load(Ordering::Relaxed),
                },
                "tts": {
                    "invocations": metrics.tts_invocations.load(Ordering::Relaxed),
                    "avg_duration_ns": metrics.tts_duration_ns.load(Ordering::Relaxed) /
                                         metrics.tts_invocations.load(Ordering::Relaxed).max(1),
                    "successes": metrics.tts_successes.load(Ordering::Relaxed),
                },
            })
        };

        let json = serde_json::to_vec(&metrics)?;
        let msg = (Op::Transcript, BytesMut::from(&json[..]));
        stream.write_all(&msg_to_frame(msg)?).await?;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

/// Handle an audio stream.
async fn handle_audio_stream(stream: UnixStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let mut framed = Framed::new(stream, SocketCodec::default());
    while let Some(result) = framed.next().await {
        match result {
            Ok((Op::Audio, payload)) => {
                let segments = split_voice_segments(&runtime.vad, payload);

        for segment in segments {
            let start_hotword = std::time::Instant::now();
            let samples: Vec<i16> = segment.chunks_exact(2)
                .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                .collect();
            if runtime.hotword.detect(&samples) {
                tracing::info!("Hotword detected");
                record_hotword(start_hotword, true);
                let start_asr = std::time::Instant::now();
                if let Ok(transcript) = runtime.asr.transcribe(&samples) {
                    record_asr(start_asr, true);
                    let transcript_json = serde_json::json!({
                        "session": uuid::Uuid::new_v4().to_string(),
                        "text": transcript,
                        "final": true,
                    });
                    // TODO: Send transcript to event socket.
                } else {
                    record_asr(start_asr, false);
                }
            } else {
                record_hotword(start_hotword, false);
            }
        }
            }
            Ok((Op::Transcript, _)) => {
                return Err(anyhow::anyhow!("Unexpected transcript from client"));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Convert message to frame.
fn msg_to_frame(msg: (Op, BytesMut)) -> anyhow::Result<BytesMut> {
    let (op, payload) = msg;
    let mut frame = BytesMut::with_capacity(4 + payload.len());
    frame.put_u8(HEADER);
    frame.put_u8(op as u8);
    frame.put_u16(payload.len() as u16);
    frame.put(payload);
    Ok(frame)
}
