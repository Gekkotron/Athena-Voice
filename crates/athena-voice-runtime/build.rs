//! Build script: (re)builds the `skills-smoke-test` crate to
//! `wasm32-wasip1` and exposes the resulting `.wasm` path to the runtime's
//! tests via the `SMOKE_TEST_WASM` env var.
//!
//! `skills-smoke-test` lives outside the cargo workspace (its target is
//! `wasm32-wasip1`), so we invoke cargo on its manifest directly. Building
//! here — rather than committing a prebuilt `.wasm` — keeps the guest ABI
//! honest: if a change to the SDK breaks the smoke skill, the runtime's own
//! build fails, not just CI.
//!
//! The `wasm32-wasip1` target must be installed (see CI setup). If missing,
//! the child `cargo build` will surface its own actionable error.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/athena-voice-runtime -> project root
    let project_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("runtime crate lives at <root>/crates/athena-voice-runtime");
    let smoke_manifest = project_root.join("skills-smoke-test").join("Cargo.toml");
    let smoke_src = project_root.join("skills-smoke-test").join("src");

    // Re-run when the smoke skill's source or manifest changes.
    println!("cargo:rerun-if-changed={}", smoke_manifest.display());
    println!("cargo:rerun-if-changed={}", smoke_src.display());
    println!("cargo:rerun-if-env-changed=SMOKE_TEST_WASM_SKIP");

    // Escape hatch: setting SMOKE_TEST_WASM_SKIP=1 skips the wasm rebuild.
    // Handy for machines without the wasm32-wasip1 target during local dev
    // where the smoke skill isn't being exercised.
    if std::env::var_os("SMOKE_TEST_WASM_SKIP").is_some() {
        return;
    }

    // Build into a dedicated target dir under OUT_DIR so we do not clobber
    // the workspace's target/, and so `cargo clean` on the runtime tidies up.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));
    let target_dir = out_dir.join("smoke-target");

    let status = Command::new(env!("CARGO"))
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip1")
        .arg("--manifest-path")
        .arg(&smoke_manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .status()
        .expect("failed to invoke cargo for skills-smoke-test");
    assert!(status.success(), "building skills-smoke-test wasm failed");

    let wasm_path = target_dir
        .join("wasm32-wasip1")
        .join("release")
        .join("skills_smoke_test.wasm");
    assert!(
        wasm_path.exists(),
        "expected smoke-test wasm at {}",
        wasm_path.display()
    );
    println!("cargo:rustc-env=SMOKE_TEST_WASM={}", wasm_path.display());
}
