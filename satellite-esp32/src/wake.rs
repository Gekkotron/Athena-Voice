//! On-device wake word via esp-sr WakeNet ("Alexa", wn9_alexa).
//!
//! S3-only: esp-sr gates all WakeNet9 models to ESP32-S3/P4, and the
//! classic ESP32 has no usable models in current esp-sr — it keeps
//! push-to-talk. The model is read from the `model` flash partition
//! (see `partitions_s3.csv`; flashed by `flash-s3.sh`).
#![cfg(all(feature = "hardware", esp32s3))]

use esp_idf_svc::sys::sr;
use log::{info, warn};

pub struct WakeNet {
    iface: *const sr::esp_wn_iface_t,
    data: *mut sr::model_iface_data_t,
    buf: Vec<i16>,
    chunk: usize,
}

// The raw pointers are owned by this struct and only touched from the
// mic thread that owns the WakeNet.
unsafe impl Send for WakeNet {}

impl WakeNet {
    /// Loads the first wake model from the `model` partition. `None`
    /// (with a warning) when the partition or model is missing, so a
    /// board flashed without the model still boots into push-to-talk.
    pub fn new() -> Option<Self> {
        unsafe {
            // WakeNet9 mallocs from PSRAM and dereferences the result
            // unchecked — verify PSRAM actually initialized first.
            if esp_idf_svc::sys::heap_caps_get_total_size(esp_idf_svc::sys::MALLOC_CAP_SPIRAM)
                == 0
            {
                warn!("wake: no PSRAM detected — push-to-talk only");
                return None;
            }
            let models = sr::esp_srmodel_init(c"model".as_ptr());
            if models.is_null() {
                warn!("wake: no `model` partition / srmodels — push-to-talk only");
                return None;
            }
            let name = sr::esp_srmodel_filter(
                models,
                c"wn".as_ptr().cast_mut(),
                core::ptr::null_mut(),
            );
            if name.is_null() {
                warn!("wake: no wakenet model in partition — push-to-talk only");
                return None;
            }
            let iface = sr::esp_wn_handle_from_name(name);
            if iface.is_null() {
                warn!("wake: no iface for model — push-to-talk only");
                return None;
            }
            let create = (*iface).create?;
            let data = create(name.cast(), sr::det_mode_t_DET_MODE_90);
            if data.is_null() {
                warn!("wake: model create failed — push-to-talk only");
                return None;
            }
            let chunk = ((*iface).get_samp_chunksize?)(data).max(1) as usize;
            info!(
                "wake: loaded {:?} (chunk {} samples)",
                std::ffi::CStr::from_ptr(name),
                chunk
            );
            Some(Self {
                iface,
                data,
                buf: Vec::with_capacity(chunk * 2),
                chunk,
            })
        }
    }

    /// Feed mic samples (s16 mono 16 kHz); true when the wake word was
    /// just detected. Buffers internally to WakeNet's chunk size.
    pub fn feed(&mut self, samples: &[i16]) -> bool {
        self.buf.extend_from_slice(samples);
        let mut detected = false;
        while self.buf.len() >= self.chunk {
            let state = unsafe {
                match (*self.iface).detect {
                    Some(detect) => detect(self.data, self.buf.as_mut_ptr()),
                    None => return false,
                }
            };
            if state == sr::wakenet_state_t_WAKENET_DETECTED {
                detected = true;
            }
            self.buf.drain(..self.chunk);
        }
        detected
    }
}

impl Drop for WakeNet {
    fn drop(&mut self) {
        unsafe {
            if let Some(destroy) = (*self.iface).destroy {
                destroy(self.data);
            }
        }
    }
}
