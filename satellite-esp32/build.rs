fn main() {
    // Compiled out for host tests (--no-default-features), where no
    // ESP-IDF checkout exists to probe.
    #[cfg(feature = "hardware")]
    embuild::espidf::sysenv::output();
}
