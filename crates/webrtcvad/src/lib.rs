//! Stub WebRTC-style voice activity detector.
//!
//! Replaces the throwaway crate a previous session generated under a temp
//! directory. `athena-voice-server` only relies on this energy heuristic for
//! now; swap in a real WebRTC VAD binding without changing the call sites.

/// Voice activity detector (energy-threshold stub).
pub struct Vad;

/// Aggressiveness modes mirroring the WebRTC VAD API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadMode {
    Quality,
    LowBitrate,
    Aggressive,
    VeryAggressive,
}

impl Vad {
    /// Returns true when the frame's mean absolute amplitude crosses a fixed
    /// speech-energy threshold.
    #[must_use]
    pub fn is_voice_segment(&self, frame: &[i16]) -> bool {
        if frame.is_empty() {
            return false;
        }
        let energy: i64 = frame.iter().map(|&s| i64::from(s).abs()).sum();
        energy / frame.len() as i64 > 300
    }
}
