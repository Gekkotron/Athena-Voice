//! Build-time configuration (`cfg.toml`, see `cfg.example.toml`) and
//! board wiring constants.

/// Populated from `cfg.toml` at build time by `toml-cfg`; falls back to
/// these defaults when the file is absent (Wi-Fi will then fail to join,
/// loudly, at boot).
#[cfg(feature = "hardware")]
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
}

// Wiring (ESP-IDF pins are typed, so the per-chip bindings live in
// main.rs `wiring()` — change them there):
//   classic ESP32 (WROOM; GPIO6–11 are internal-flash pins, avoid them):
//     INMP441 mic  (I2S0 RX): SCK→GPIO32, WS→GPIO25, SD→GPIO33, L/R→GND
//     MAX98357A amp (I2S1 TX): BCLK→GPIO27, LRC→GPIO26, DIN→GPIO22
//   ESP32-S3:
//     INMP441 mic  (I2S0 RX): SCK→GPIO4, WS→GPIO5, SD→GPIO6, L/R→GND
//     MAX98357A amp (I2S1 TX): BCLK→GPIO15, LRC→GPIO16, DIN→GPIO7
//   both: BOOT button (GPIO0, active low, internal pull-up) = push-to-talk
