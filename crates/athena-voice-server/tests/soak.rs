//! Server-side audio front-end tests.
//!
//! The original 24-hour soak concept (stream WAV clips at the audio socket
//! and assert hotword → transcript round-trips) needs the audio socket to be
//! implemented first (`socket::start_audio_socket` is still a stub). Until
//! then this file covers the pieces that do exist: the VAD detector and
//! voice segmentation.

use athena_voice_server::vad::{VadDetector, split_voice_segments};
use bytes::BytesMut;

fn pcm_bytes(samples: &[i16]) -> BytesMut {
    let mut buf = BytesMut::with_capacity(samples.len() * 2);
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

#[test]
fn silence_yields_no_voice_segments() {
    let vad = VadDetector::new(2).expect("vad");
    // 100 ms of silence at 48 kHz mono.
    let silence = vec![0i16; 4_800];
    let segments = split_voice_segments(&vad, pcm_bytes(&silence));
    assert!(segments.is_empty(), "silence must not produce segments");
}

#[test]
fn loud_signal_yields_voice_segments() {
    let vad = VadDetector::new(2).expect("vad");
    // 100 ms square wave well above the energy threshold.
    let loud: Vec<i16> = (0..4_800)
        .map(|i| if i % 2 == 0 { 12_000 } else { -12_000 })
        .collect();
    let segments = split_voice_segments(&vad, pcm_bytes(&loud));
    assert!(!segments.is_empty(), "loud signal must produce segments");
    let total: usize = segments.iter().map(BytesMut::len).sum();
    assert!(total > 0);
}

#[test]
fn short_buffer_is_not_voice() {
    let vad = VadDetector::new(2).expect("vad");
    // Shorter than one 10 ms frame at 48 kHz.
    assert!(!vad.process(&[10_000i16; 100]));
}
