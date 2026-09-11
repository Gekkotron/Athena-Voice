//! Build-time configuration (`cfg.toml`, see `cfg.example.toml`) and
//! board wiring constants.

/// Populated from `cfg.toml` at build time by `toml-cfg`; falls back to
/// these defaults when the file is absent (Wi-Fi will then fail to join,
/// loudly, at boot).
#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_pass: &'static str,
    #[default("mqtt://127.0.0.1:1883")]
    mqtt_url: &'static str,
    #[default("esp32-sat")]
    sat_id: &'static str,
    #[default("fr")]
    locale: &'static str,
    /// Must match the server's `[mqtt] topic_root`. A mismatch is silent:
    /// the satellite publishes into a namespace nothing subscribes to.
    #[default("assist")]
    topic_root: &'static str,
}

/// Longest plausible value for any config string. A `&'static str`
/// reporting more than this means the baked-in constant is not the one
/// this source generates — a stale or partially-written flash. Reading
/// `.len()` is safe (it is just the fat pointer's second word), while
/// *formatting* such a string panics deep inside `format!` with
/// "capacity overflow", far from the real cause.
const MAX_LEN: usize = 512;

/// Returns `value` when its length is plausible, else `fallback` — so a
/// corrupt constant degrades to a working default with a named warning
/// instead of aborting the boot.
#[must_use]
pub fn checked(value: &'static str, name: &str, fallback: &'static str) -> &'static str {
    if value.len() > MAX_LEN {
        log::error!(
            "config field `{name}` is corrupt (len {}) — using {fallback:?}. \
             Reflash from a clean build: the running binary does not match \
             the source that generated it.",
            value.len()
        );
        return fallback;
    }
    value
}

/// Logs every field's length at boot (lengths only — never the values,
/// so a corrupt string cannot panic the logger, and no secret is
/// printed). The first line of defence when the device misbehaves.
pub fn log_lengths() {
    log::info!(
        "config lengths: ssid={} pass={} url={} sat_id={} locale={} topic_root={}",
        CONFIG.wifi_ssid.len(),
        CONFIG.wifi_pass.len(),
        CONFIG.mqtt_url.len(),
        CONFIG.sat_id.len(),
        CONFIG.locale.len(),
        CONFIG.topic_root.len(),
    );
}

// Wiring (ESP-IDF pins are typed, so the per-chip bindings live in
// main.rs `wiring()` — change them there):
//   classic ESP32 (WROOM; GPIO6–11 are internal-flash pins, avoid them):
//     INMP441 mic  (I2S0 RX): SCK→GPIO32, WS→GPIO25, SD→GPIO33, L/R→GND
//     MAX98357A amp (I2S1 TX): BCLK→GPIO27, LRC→GPIO26, DIN→GPIO22
//   ESP32-S3 (pins also on the XIAO ESP32S3 header: D3-D5 / D8-D10):
//     INMP441 mic  (I2S0 RX): SCK→GPIO4, WS→GPIO5, SD→GPIO6, L/R→GND
//     MAX98357A amp (I2S1 TX): BCLK→GPIO7, LRC→GPIO8, DIN→GPIO9
//   both: BOOT button (GPIO0, active low, internal pull-up) = push-to-talk

#[cfg(test)]
mod tests {
    use super::{CONFIG, checked};

    /// The generated constant must hold sane `&'static str`s. A field
    /// added to `Config` without a working `#[default(...)]` (or a
    /// toml-cfg substitution gone wrong) shows up here as a garbage
    /// length rather than as a capacity-overflow panic on the device.
    #[test]
    fn generated_config_strings_are_sane() {
        for (name, value) in [
            ("wifi_ssid", CONFIG.wifi_ssid),
            ("wifi_pass", CONFIG.wifi_pass),
            ("mqtt_url", CONFIG.mqtt_url),
            ("sat_id", CONFIG.sat_id),
            ("locale", CONFIG.locale),
            ("topic_root", CONFIG.topic_root),
        ] {
            assert!(
                value.len() < 512,
                "{name} has an implausible length ({})",
                value.len()
            );
            assert!(value.is_ascii(), "{name} is not ascii: {value:?}");
        }
        assert!(!CONFIG.topic_root.is_empty(), "topic_root must not be empty");
        assert!(!CONFIG.sat_id.is_empty(), "sat_id must not be empty");
    }

    #[test]
    fn checked_passes_plausible_values_through() {
        assert_eq!(checked("assist", "topic_root", "athena"), "assist");
        assert_eq!(checked("", "topic_root", "athena"), "");
    }

    #[test]
    fn checked_substitutes_the_fallback_for_an_implausible_length() {
        // A corrupt fat pointer reports a huge length; `checked` must
        // hand back the fallback rather than let format! blow up.
        let huge: &'static str = unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                b"x".as_ptr(),
                super::MAX_LEN + 1,
            ))
        };
        assert_eq!(checked(huge, "topic_root", "athena"), "athena");
    }
}
