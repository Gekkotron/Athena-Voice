fn main() {
    // The chip cfgs main.rs switches wiring on; esp-idf-sys emits only
    // the active chip's cfg, so declare both to keep check-cfg quiet.
    println!("cargo::rustc-check-cfg=cfg(esp32)");
    println!("cargo::rustc-check-cfg=cfg(esp32s3)");
    // Compiled out for host tests (--no-default-features), where no
    // ESP-IDF checkout exists to probe.
    #[cfg(feature = "hardware")]
    embuild::espidf::sysenv::output();
}
