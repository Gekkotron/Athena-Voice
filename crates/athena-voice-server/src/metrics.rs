//! Runtime metrics for VAD, ASR, TTS, and hotword detection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Global metrics.
#[derive(Debug, Default)]
pub struct Metrics {
    pub vad_invocations: AtomicU64,
    pub vad_duration_ns: AtomicU64,
    pub hotword_invocations: AtomicU64,
    pub hotword_duration_ns: AtomicU64,
    pub asr_invocations: AtomicU64,
    pub asr_duration_ns: AtomicU64,
    pub tts_invocations: AtomicU64,
    pub tts_duration_ns: AtomicU64,
    pub hotword_detections: AtomicU64,
    pub asr_successes: AtomicU64,
    pub tts_successes: AtomicU64,
}

/// Record VAD processing time.
pub fn record_vad(start: Instant) {
    let ns = start.elapsed().as_nanos() as u64;
    let metrics = METRICS.get_or_init(Metrics::default);
    metrics.vad_invocations.fetch_add(1, Ordering::Relaxed);
    metrics.vad_duration_ns.fetch_add(ns, Ordering::Relaxed);
}

/// Record hotword processing time and result.
pub fn record_hotword(start: Instant, detected: bool) {
    let ns = start.elapsed().as_nanos() as u64;
    let metrics = METRICS.get_or_init(Metrics::default);
    metrics.hotword_invocations.fetch_add(1, Ordering::Relaxed);
    metrics.hotword_duration_ns.fetch_add(ns, Ordering::Relaxed);
    if detected {
        metrics.hotword_detections.fetch_add(1, Ordering::Relaxed);
    }
}

/// Record ASR processing time and result.
pub fn record_asr(start: Instant, success: bool) {
    let ns = start.elapsed().as_nanos() as u64;
    let metrics = METRICS.get_or_init(Metrics::default);
    metrics.asr_invocations.fetch_add(1, Ordering::Relaxed);
    metrics.asr_duration_ns.fetch_add(ns, Ordering::Relaxed);
    if success {
        metrics.asr_successes.fetch_add(1, Ordering::Relaxed);
    }
}

/// Record TTS processing time and result.
pub fn record_tts(start: Instant, success: bool) {
    let ns = start.elapsed().as_nanos() as u64;
    let metrics = METRICS.get_or_init(Metrics::default);
    metrics.tts_invocations.fetch_add(1, Ordering::Relaxed);
    metrics.tts_duration_ns.fetch_add(ns, Ordering::Relaxed);
    if success {
        metrics.tts_successes.fetch_add(1, Ordering::Relaxed);
    }
}

/// Global metrics instance.
static METRICS: once_cell::sync::OnceCell<Metrics> = once_cell::sync::OnceCell::new();
