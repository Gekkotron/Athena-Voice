//! Plan 8 — skill-friendly INI-style config.
//!
//! The host serves the bytes of `[skills] config_file` verbatim through
//! `host_config_get` (covered by unit tests in `wasm::host_fns`); this test
//! locks in the guest-side contract: the SDK's `IniSlice` parses those bytes
//! into sections a skill can query.

use athena_voice_skill_sdk::host::IniSlice;

#[test]
fn ini_section_lookup_roundtrip() {
    let ini = "[audio]\nvolume = 0.7\nspeed = fast\n\n[net]\ntimeout = 5\n";
    let slice = IniSlice::from_bytes(ini.as_bytes().to_vec());

    let audio = slice.section("audio").expect("audio section present");
    assert_eq!(
        audio.get("volume").cloned().flatten().as_deref(),
        Some("0.7")
    );
    assert_eq!(
        audio.get("speed").cloned().flatten().as_deref(),
        Some("fast")
    );

    let net = slice.section("net").expect("net section present");
    assert_eq!(net.get("timeout").cloned().flatten().as_deref(), Some("5"));

    assert!(slice.section("missing").is_none());
}

#[test]
fn ini_garbage_bytes_yield_no_sections() {
    let slice = IniSlice::from_bytes(vec![0xFF, 0xFE, 0x00]);
    assert!(slice.section("audio").is_none());
}
