//! INMP441 capture: I2S driver plus the host-testable sample conversion
//! and end-of-utterance silence tracking.

/// Samples per 20 ms frame at 16 kHz.
pub const FRAME_SAMPLES: usize = 320;
/// Raw I2S bytes per frame (32-bit slots).
pub const FRAME_RAW_BYTES: usize = FRAME_SAMPLES * 4;
/// Spec default: 800 ms of sustained silence ends the utterance.
pub const SILENCE_MS: u32 = 800;
/// RMS amplitude below which a frame counts as silence.
pub const SILENCE_RMS: u32 = 500;

/// The INMP441 delivers 24-bit samples left-justified in 32-bit slots.
/// Convert raw little-endian i32 slots to s16, shifting by 14 so the
/// low noise bits drop and ~2 bits of headroom act as fixed gain,
/// clamped rather than wrapped.
pub fn convert_inmp441(raw: &[u8], out: &mut Vec<i16>) {
    out.clear();
    for slot in raw.chunks_exact(4) {
        let v = i32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]) >> 14;
        out.push(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16);
    }
}

/// Detects end of utterance: `push` returns true once speech has been
/// heard and frames have then stayed under the silence threshold for
/// `silence_ms` in a row.
///
/// Silence *before* speech never ends an utterance — between the wake
/// word and the question there is a natural pause, and counting it would
/// close the recording before the user starts talking. The 10 s cap in
/// `session.rs` bounds the "user never spoke" case.
///
/// The threshold adapts to the room: a rolling noise-floor estimate
/// (drops instantly to quieter frames, creeps up slowly — doubling in
/// roughly a minute — so speech can't capture it) sets the bar at
/// `3 x floor`, never below `min_threshold`. Real INMP441 quiet-room
/// levels run rms 700-1300, so any fixed number is wrong somewhere.
pub struct SilenceTracker {
    min_threshold: u32,
    silence_ms: u32,
    frame_ms: u32,
    run_ms: u32,
    floor: Option<u32>,
    seen_frames: u32,
    speech_started: bool,
}

/// Frames ignored before floor learning starts — the DC blocker's
/// startup transient reads near-zero and must not pin the floor.
const FLOOR_WARMUP_FRAMES: u32 = 10;

impl SilenceTracker {
    pub fn new(min_threshold: u32, silence_ms: u32, frame_ms: u32) -> Self {
        Self {
            min_threshold,
            silence_ms,
            frame_ms,
            run_ms: 0,
            floor: None,
            seen_frames: 0,
            speech_started: false,
        }
    }

    /// Starts a new utterance; the learned noise floor is kept, but the
    /// speech gate re-arms.
    pub fn reset(&mut self) {
        self.run_ms = 0;
        self.speech_started = false;
    }

    /// Whether speech has been detected in the current utterance.
    pub fn speech_started(&self) -> bool {
        self.speech_started
    }

    /// Update the noise-floor estimate from an idle-state frame without
    /// running silence detection.
    pub fn observe(&mut self, frame: &[i16]) {
        self.update_floor(rms(frame));
    }

    pub fn push(&mut self, frame: &[i16]) -> bool {
        let r = rms(frame);
        // Threshold from the floor as it stood BEFORE this frame — a
        // cold-start speech frame must not set its own bar.
        let threshold = self.threshold();
        self.update_floor(r);
        if r >= threshold {
            self.speech_started = true;
            self.run_ms = 0;
            return false;
        }
        if !self.speech_started {
            // Still waiting for the user to start talking.
            return false;
        }
        self.run_ms += self.frame_ms;
        self.run_ms >= self.silence_ms
    }

    fn threshold(&self) -> u32 {
        self.floor
            .map_or(0, |f| f.saturating_mul(3))
            .max(self.min_threshold)
    }

    fn update_floor(&mut self, r: u32) {
        self.seen_frames = self.seen_frames.saturating_add(1);
        if self.seen_frames <= FLOOR_WARMUP_FRAMES {
            return;
        }
        self.floor = Some(match self.floor {
            None => r.max(1),
            // Down: asymmetric EMA — one quiet dip only nudges the
            // floor 1/8 of the way, a sustained quieter room converges
            // in well under a second.
            Some(f) if r < f => f - (f - r).div_ceil(8),
            // Up: slow creep (doubling ≈ a minute) so speech can't
            // capture the floor.
            Some(f) => f + (f / 4096).max(1),
        });
    }
}

fn rms(frame: &[i16]) -> u32 {
    if frame.is_empty() {
        return 0;
    }
    let sum_sq: u64 = frame
        .iter()
        .map(|&s| {
            let s = i64::from(s);
            (s * s) as u64
        })
        .sum();
    isqrt(sum_sq / frame.len() as u64) as u32
}

/// Integer square root (u64), no floats on the Xtensa FPU path.
fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// One-pole DC-blocking high-pass (cutoff ≈ 10 Hz at 16 kHz):
/// `y[n] = x[n] - x[n-1] + a·y[n-1]` with `a = 255/256`, integer math.
/// The INMP441 carries a large, slowly drifting DC bias that otherwise
/// keeps "silence" far above the end-of-utterance RMS threshold.
pub struct DcBlocker {
    prev_x: i32,
    /// y in Q8 fixed point — plain integer `y -= y>>8` stalls below 256
    /// and would leave a residual offset of up to 255.
    y_q8: i32,
}

impl DcBlocker {
    pub fn new() -> Self {
        Self { prev_x: 0, y_q8: 0 }
    }

    pub fn process(&mut self, samples: &mut [i16]) {
        for s in samples {
            let x = i32::from(*s);
            self.y_q8 = ((x - self.prev_x) << 8) + self.y_q8 - (self.y_q8 >> 8);
            self.prev_x = x;
            *s = (self.y_q8 >> 8).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        }
    }
}

/// Aggregates raw mic levels over a window of frames — the serial
/// monitor's "is the microphone wired right?" display. `push` returns
/// `Some((max_frame_rms, peak_sample))` once per window, then resets.
pub struct LevelMeter {
    window: u32,
    frames: u32,
    rms_max: u32,
    peak: u16,
}

impl LevelMeter {
    pub fn new(window_frames: u32) -> Self {
        Self {
            window: window_frames.max(1),
            frames: 0,
            rms_max: 0,
            peak: 0,
        }
    }

    pub fn push(&mut self, frame: &[i16]) -> Option<(u32, u16)> {
        self.rms_max = self.rms_max.max(rms(frame));
        self.peak = self
            .peak
            .max(frame.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0));
        self.frames += 1;
        if self.frames < self.window {
            return None;
        }
        let out = (self.rms_max, self.peak);
        self.frames = 0;
        self.rms_max = 0;
        self.peak = 0;
        Some(out)
    }
}

/// I2S capture driver for the INMP441 (hardware only).
#[cfg(feature = "hardware")]
pub mod driver {
    use esp_idf_svc::hal::gpio;
    use esp_idf_svc::hal::i2s::config::{
        Config, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig, StdSlotConfig,
    };
    use esp_idf_svc::hal::i2s::{I2S0, I2sDriver, I2sRx};
    use esp_idf_svc::sys::EspError;

    use super::{FRAME_RAW_BYTES, convert_inmp441};

    pub struct Mic {
        driver: I2sDriver<'static, I2sRx>,
        raw: Vec<u8>,
        samples: Vec<i16>,
        dc: super::DcBlocker,
    }

    impl Mic {
        /// Pins are `AnyIOPin` so `main.rs` can bind different GPIOs per
        /// chip (classic ESP32 vs S3).
        pub fn new(
            i2s: I2S0,
            bclk: gpio::AnyIOPin,
            ws: gpio::AnyIOPin,
            din: gpio::AnyIOPin,
        ) -> Result<Self, EspError> {
            let cfg = StdConfig::new(
                Config::default(),
                StdClkConfig::from_sample_rate_hz(16_000),
                StdSlotConfig::philips_slot_default(DataBitWidth::Bits32, SlotMode::Mono),
                StdGpioConfig::default(),
            );
            let mut driver =
                I2sDriver::new_std_rx(i2s, &cfg, bclk, din, gpio::AnyIOPin::none(), ws)?;
            driver.rx_enable()?;
            Ok(Self {
                driver,
                raw: vec![0u8; FRAME_RAW_BYTES],
                samples: Vec::with_capacity(super::FRAME_SAMPLES),
                dc: super::DcBlocker::new(),
            })
        }

        /// Blocking read of one ~20 ms frame, converted to s16le bytes
        /// appended into `out` (cleared first). Also returns the samples
        /// for the silence tracker.
        pub fn read_frame(&mut self, out: &mut Vec<u8>) -> Result<&[i16], EspError> {
            let mut filled = 0;
            while filled < self.raw.len() {
                filled += self.driver.read(&mut self.raw[filled..], u32::MAX)?;
            }
            convert_inmp441(&self.raw, &mut self.samples);
            self.dc.process(&mut self.samples);
            out.clear();
            for s in &self.samples {
                out.extend_from_slice(&s.to_le_bytes());
            }
            Ok(&self.samples)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_shifts_32bit_slots_to_i16() {
        let raw = (1i32 << 20).to_le_bytes();
        let mut out = Vec::new();
        convert_inmp441(&raw, &mut out);
        assert_eq!(out, vec![64i16]);
    }

    #[test]
    fn convert_clamps_instead_of_wrapping() {
        let raw = i32::MAX.to_le_bytes();
        let mut out = Vec::new();
        convert_inmp441(&raw, &mut out);
        assert_eq!(out, vec![i16::MAX]);
        let raw = i32::MIN.to_le_bytes();
        out.clear();
        convert_inmp441(&raw, &mut out);
        assert_eq!(out, vec![i16::MIN]);
    }

    #[test]
    fn convert_clears_previous_output() {
        let raw = 0i32.to_le_bytes();
        let mut out = vec![7i16; 3];
        convert_inmp441(&raw, &mut out);
        assert_eq!(out, vec![0i16]);
    }

    #[test]
    fn dc_blocker_removes_constant_offset() {
        let mut dc = DcBlocker::new();
        let mut frame = vec![5000i16; 320];
        // Settle over a few frames (one-pole filter decays exponentially).
        for _ in 0..10 {
            frame.fill(5000);
            dc.process(&mut frame);
        }
        assert!(rms(&frame) < 100, "residual rms {} too high", rms(&frame));
    }

    #[test]
    fn dc_blocker_passes_audio_band_signal() {
        let mut dc = DcBlocker::new();
        // 8 kHz square wave at 16 kHz sample rate, on top of a DC offset.
        let mut frame: Vec<i16> = (0..320)
            .map(|i| if i % 2 == 0 { 8000 } else { 2000 })
            .collect();
        for _ in 0..10 {
            for (i, s) in frame.iter_mut().enumerate() {
                *s = if i % 2 == 0 { 8000 } else { 2000 };
            }
            dc.process(&mut frame);
        }
        // The ±3000 AC component survives; the 5000 DC midpoint is gone.
        let r = rms(&frame);
        assert!(r > 2500 && r < 3500, "ac rms {r} out of range");
    }

    #[test]
    fn level_meter_reports_window_max_and_resets() {
        let mut meter = LevelMeter::new(3);
        let quiet = vec![10i16; 4];
        let loud = vec![-3000i16, 3000, -3000, 3000];
        assert_eq!(meter.push(&quiet), None);
        assert_eq!(meter.push(&loud), None);
        let (rms, peak) = meter.push(&quiet).expect("window complete");
        assert_eq!(peak, 3000);
        assert_eq!(rms, 3000); // max frame RMS in the window
        // Next window starts fresh.
        assert_eq!(meter.push(&quiet), None);
        assert_eq!(meter.push(&quiet), None);
        let (rms, peak) = meter.push(&quiet).expect("window complete");
        assert_eq!((rms, peak), (10, 10));
    }

    #[test]
    fn startup_transient_does_not_pin_the_floor() {
        let mut t = SilenceTracker::new(500, 800, 20);
        // DC-blocker settling: near-zero frames right after boot.
        let transient = vec![2i16; 320];
        let ambient = vec![800i16; 320];
        for _ in 0..10 {
            t.observe(&transient);
        }
        for _ in 0..20 {
            t.observe(&ambient);
        }
        // Floor must reflect the ambient 800, not the transient ~0: with
        // a correct floor the threshold sits at 2400, so ambient counts
        // as silence. (A floor pinned near 0 would put the threshold at
        // the 500 minimum and read ambient as *speech*, which never
        // ends.) The speech frame arms the gate first.
        t.push(&vec![8000i16; 320]);
        for _ in 0..39 {
            assert!(!t.push(&ambient));
        }
        assert!(t.push(&ambient));
    }

    #[test]
    fn single_quiet_dip_does_not_pin_the_floor() {
        let mut t = SilenceTracker::new(500, 800, 20);
        let ambient = vec![800i16; 320];
        let dip = vec![50i16; 320];
        for _ in 0..20 {
            t.observe(&ambient); // past warmup, floor ≈ 800
        }
        t.observe(&dip); // one anomalously quiet frame
        // Ambient must still count as silence afterwards.
        t.push(&vec![8000i16; 320]); // arm the speech gate
        for _ in 0..39 {
            assert!(!t.push(&ambient));
        }
        assert!(t.push(&ambient));
    }

    #[test]
    fn silence_before_speech_never_ends_the_utterance() {
        // The gap between the wake word and the question: the user has
        // not started talking yet, so no amount of quiet may close the
        // utterance (this used to cut recordings at ~1.1 s).
        let mut t = SilenceTracker::new(500, 800, 20);
        let ambient = vec![800i16; 320];
        for _ in 0..20 {
            t.observe(&ambient); // warm up the floor while idle
        }
        for _ in 0..250 {
            // 5 s of silence, far past silence_ms
            assert!(!t.push(&ambient), "ended before speech started");
        }
        // Speech arrives, then stops: now the rule applies.
        let speech = vec![8000i16; 320];
        for _ in 0..10 {
            assert!(!t.push(&speech));
        }
        for _ in 0..39 {
            assert!(!t.push(&ambient));
        }
        assert!(t.push(&ambient), "must end 800 ms after speech stopped");
    }

    #[test]
    fn reset_requires_speech_again() {
        // Each utterance starts fresh: a new session must not inherit
        // the previous one's "speech already started" state.
        let mut t = SilenceTracker::new(500, 800, 20);
        let ambient = vec![800i16; 320];
        let speech = vec![8000i16; 320];
        for _ in 0..20 {
            t.observe(&ambient);
        }
        t.push(&speech);
        for _ in 0..40 {
            t.push(&ambient);
        }
        t.reset();
        for _ in 0..100 {
            assert!(!t.push(&ambient), "reset must re-arm the speech gate");
        }
    }

    #[test]
    fn tracker_learns_ambient_noise_floor_as_silence() {
        let mut t = SilenceTracker::new(500, 800, 20);
        // A quiet room that still reads rms≈800 (real INMP441 levels).
        let ambient = vec![800i16; 320];
        let speech = vec![8000i16; 320];
        for _ in 0..20 {
            t.observe(&ambient); // idle: warmup, then learn the floor
        }
        // Speech clearly above 3x floor resets the run...
        assert!(!t.push(&speech));
        // ...and the ambient floor now counts as silence.
        for _ in 0..39 {
            assert!(!t.push(&ambient));
        }
        assert!(t.push(&ambient));
    }

    #[test]
    fn tracker_fires_after_sustained_silence() {
        let mut t = SilenceTracker::new(500, 800, 20);
        let loud = vec![2000i16; 320];
        let quiet = vec![10i16; 320];
        assert!(!t.push(&loud));
        for _ in 0..39 {
            assert!(!t.push(&quiet)); // 780 ms of silence so far
        }
        assert!(t.push(&quiet)); // 800 ms reached
    }

    #[test]
    fn loud_frame_resets_the_silence_run() {
        let mut t = SilenceTracker::new(500, 800, 20);
        let loud = vec![2000i16; 320];
        let quiet = vec![10i16; 320];
        for _ in 0..30 {
            t.push(&quiet);
        }
        t.push(&loud);
        for _ in 0..39 {
            assert!(!t.push(&quiet));
        }
        assert!(t.push(&quiet));
    }

    #[test]
    fn reset_clears_the_run() {
        let mut t = SilenceTracker::new(500, 40, 20);
        let quiet = vec![0i16; 320];
        let speech = vec![8000i16; 320];
        t.push(&speech);
        assert!(!t.push(&quiet)); // 20 ms of silence banked
        t.reset();
        // The banked silence is gone AND the gate re-armed, so the next
        // utterance needs speech before any silence counts.
        t.push(&speech);
        assert!(!t.push(&quiet));
        assert!(t.push(&quiet));
    }
}
