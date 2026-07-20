//! Voice Activity Detection using WebRTC VAD.

use std::sync::Arc;

use crate::metrics::record_vad;
use bytes::BytesMut;
use std::time::Instant;
use webrtcvad::{Vad, VadMode};

/// Voice Activity Detector.
#[derive(Clone)]
pub struct VadDetector {
    vad: Arc<Vad>,
    sample_rate: u32,
    frame_ms: u32,
}

impl VadDetector {
    /// Create a new VAD detector (stub).
    pub fn new(aggressiveness: u8) -> anyhow::Result<Self> {
        let _ = aggressiveness;
        Ok(Self {
            vad: Arc::new(Vad),
            sample_rate: 48000,
            frame_ms: 10,
        })
    }

    /// Process audio data and return whether voice is detected.
    pub fn process(&self, audio: &[i16]) -> bool {
        let start = Instant::now();
        let frame_len = (self.sample_rate / 1000 * self.frame_ms) as usize;
        if audio.len() < frame_len {
            record_vad(start);
            return false;
        }

        // Split into 10ms frames (mono).
        let frame = &audio[..frame_len];
        let detected = self.vad.is_voice_segment(frame);
        record_vad(start);
        detected
    }
}

/// Split audio into voice segments.
pub fn split_voice_segments(vad: &VadDetector, audio: BytesMut) -> Vec<BytesMut> {
    let mut segments: Vec<BytesMut> = Vec::new();
    let frame_len = (vad.sample_rate / 1000 * vad.frame_ms) as usize * 2; // 2 bytes per sample
    let mut cursor = 0;

    while cursor + frame_len <= audio.len() {
        let frame = &audio[cursor..cursor + frame_len];
        let samples = frame
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();

        if vad.process(&samples) {
            // Grow the current segment.
            if let Some(last) = segments.last_mut() {
                last.extend_from_slice(frame);
            } else {
                segments.push(BytesMut::from(frame));
            }
        }

        cursor += frame_len;
    }

    segments
}
