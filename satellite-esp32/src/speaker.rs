//! MAX98357A playback: a dedicated thread owns the I2S TX driver and
//! drains a queue of s16le chunks, rebuilding the driver when tts/meta
//! announces a different sample rate.
#![cfg(feature = "hardware")]

use std::sync::mpsc::{Sender, channel};

use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::i2s::config::{
    Config, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig, StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2S1, I2sDriver, I2sTx};
use esp_idf_svc::sys::EspError;
use log::warn;

/// The runtime's TTS default (see tts/meta).
const DEFAULT_RATE: u32 = 24_000;

enum Msg {
    Rate(u32),
    Chunk(Vec<u8>),
}

pub struct Speaker {
    tx: Sender<Msg>,
}

impl Speaker {
    /// Pins are `AnyIOPin` so `main.rs` can bind different GPIOs per
    /// chip (classic ESP32 vs S3).
    pub fn new(
        i2s: I2S1,
        bclk: gpio::AnyIOPin,
        lrc: gpio::AnyIOPin,
        dout: gpio::AnyIOPin,
    ) -> Result<Self, EspError> {
        let (tx, rx) = channel::<Msg>();
        // The gpio/i2s handles aren't Send-friendly to re-create per rate
        // change from outside, so the playback thread owns them for the
        // process lifetime and rebuilds the driver in place.
        std::thread::Builder::new()
            .name("speaker".into())
            .stack_size(8 * 1024)
            .spawn(move || {
                let mut i2s = i2s;
                let mut bclk = bclk;
                let mut lrc = lrc;
                let mut dout = dout;
                let mut rate = DEFAULT_RATE;
                loop {
                    let mut driver = match make_driver(&mut i2s, &mut bclk, &mut lrc, &mut dout, rate)
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!("speaker i2s init failed ({e}); retrying in 1 s");
                            std::thread::sleep(std::time::Duration::from_secs(1));
                            continue;
                        }
                    };
                    loop {
                        match rx.recv() {
                            Ok(Msg::Rate(r)) if r != rate => {
                                rate = r;
                                break; // drop the driver, rebuild at the new rate
                            }
                            Ok(Msg::Rate(_)) => {}
                            Ok(Msg::Chunk(bytes)) => {
                                if let Err(e) = driver.write_all(&bytes, u32::MAX) {
                                    warn!("speaker write failed: {e}");
                                }
                            }
                            Err(_) => return, // Speaker dropped; end thread
                        }
                    }
                    drop(driver);
                }
            })
            .map_err(|_| EspError::from_infallible::<{ esp_idf_svc::sys::ESP_FAIL }>())?;
        Ok(Self { tx })
    }

    /// From tts/meta; rebuilds the driver if the rate changed.
    pub fn set_sample_rate(&self, rate: u32) {
        let _ = self.tx.send(Msg::Rate(rate));
    }

    /// Enqueue one s16le mono chunk; played in order.
    pub fn enqueue(&self, chunk: Vec<u8>) {
        let _ = self.tx.send(Msg::Chunk(chunk));
    }
}

fn make_driver<'a>(
    i2s: &'a mut I2S1,
    bclk: &'a mut gpio::AnyIOPin,
    lrc: &'a mut gpio::AnyIOPin,
    dout: &'a mut gpio::AnyIOPin,
    rate: u32,
) -> Result<I2sDriver<'a, I2sTx>, EspError> {
    let cfg = StdConfig::new(
        Config::default(),
        StdClkConfig::from_sample_rate_hz(rate),
        StdSlotConfig::philips_slot_default(DataBitWidth::Bits16, SlotMode::Mono),
        StdGpioConfig::default(),
    );
    let mut driver = I2sDriver::new_std_tx(
        &mut *i2s,
        &cfg,
        &mut *bclk,
        &mut *dout,
        gpio::AnyIOPin::none(),
        &mut *lrc,
    )?;
    driver.tx_enable()?;
    Ok(driver)
}
