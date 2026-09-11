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

/// Detects end of utterance: `push` returns true once frames have stayed
/// under the RMS threshold for `silence_ms` in a row.
pub struct SilenceTracker {
    threshold: u32,
    silence_ms: u32,
    frame_ms: u32,
    run_ms: u32,
}

impl SilenceTracker {
    pub fn new(threshold: u32, silence_ms: u32, frame_ms: u32) -> Self {
        Self {
            threshold,
            silence_ms,
            frame_ms,
            run_ms: 0,
        }
    }

    pub fn reset(&mut self) {
        self.run_ms = 0;
    }

    pub fn push(&mut self, frame: &[i16]) -> bool {
        if rms(frame) < self.threshold {
            self.run_ms += self.frame_ms;
        } else {
            self.run_ms = 0;
        }
        self.run_ms >= self.silence_ms
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
        assert!(!t.push(&quiet));
        t.reset();
        assert!(!t.push(&quiet));
        assert!(t.push(&quiet));
    }
}
