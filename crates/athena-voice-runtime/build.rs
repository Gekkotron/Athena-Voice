//! Build script: (re)builds the `skills-smoke-test` and `skills-timer`
//! crates to `wasm32-wasip1` and exposes the resulting `.wasm` paths to the
//! runtime's tests via the `SMOKE_TEST_WASM` / `TIMER_TEST_WASM` env vars.
//!
//! Both crates live outside the cargo workspace (their target is
//! `wasm32-wasip1`), so we invoke cargo on their manifests directly.
//! Building here — rather than committing a prebuilt `.wasm` — keeps the
//! guest ABI honest: if a change to the SDK breaks a skill, the runtime's own
//! build fails, not just CI.
//!
//! The `wasm32-wasip1` target must be installed (see CI setup). If missing,
//! the child `cargo build` will surface its own actionable error.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/athena-voice-runtime -> project root
    let project_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("runtime crate lives at <root>/crates/athena-voice-runtime");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by cargo"));

    // Every skill links the SDK as a path dep — a guest-ABI change there must
    // re-trigger the wasm rebuilds below.
    println!(
        "cargo:rerun-if-changed={}",
        project_root
            .join("crates/athena-voice-skill-sdk/src")
            .display()
    );

    build_skill_wasm(
        project_root,
        &out_dir,
        "skills-smoke-test",
        "skills_smoke_test",
        "smoke-target",
        "SMOKE_TEST_WASM",
        "SMOKE_TEST_WASM_SKIP",
    );
    build_skill_wasm(
        project_root,
        &out_dir,
        "skills-timer",
        "skills_timer",
        "timer-target",
        "TIMER_TEST_WASM",
        "TIMER_TEST_WASM_SKIP",
    );
    build_skill_wasm(
        project_root,
        &out_dir,
        "skills-home",
        "skills_home",
        "home-target",
        "HOME_TEST_WASM",
        "HOME_TEST_WASM_SKIP",
    );
    build_skill_wasm(
        project_root,
        &out_dir,
        "skills-jeedom",
        "skills_jeedom",
        "jeedom-target",
        "JEEDOM_TEST_WASM",
        "JEEDOM_TEST_WASM_SKIP",
    );
    build_skill_wasm(
        project_root,
        &out_dir,
        "skills-weather",
        "skills_weather",
        "weather-target",
        "WEATHER_TEST_WASM",
        "WEATHER_TEST_WASM_SKIP",
    );
}

/// Builds `<project_root>/<crate_dir>` to `wasm32-wasip1` and exposes the
/// resulting `.wasm` path via `cargo:rustc-env=<env_var>=<path>`.
///
/// `skip_env` is an escape hatch: setting it skips the wasm rebuild — handy
/// for machines without the `wasm32-wasip1` target during local dev where
/// the skill isn't being exercised.
#[allow(clippy::too_many_arguments)]
fn build_skill_wasm(
    project_root: &Path,
    out_dir: &Path,
    crate_dir: &str,
    lib_name: &str,
    target_subdir: &str,
    env_var: &str,
    skip_env: &str,
) {
    let manifest = project_root.join(crate_dir).join("Cargo.toml");
    let src = project_root.join(crate_dir).join("src");

    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed={skip_env}");

    if std::env::var_os(skip_env).is_some() {
        return;
    }

    // Build into a dedicated target dir under OUT_DIR so we do not clobber
    // the workspace's target/, and so `cargo clean` on the runtime tidies up.
    let target_dir = out_dir.join(target_subdir);

    // Clear RUSTFLAGS-style env inherited from the parent so tools like
    // `cargo llvm-cov` don't leak `-C instrument-coverage` into the wasm
    // build — that flag pulls in `profiler_builtins`, which wasm32-wasip1
    // has no target support for.
    let status = Command::new(env!("CARGO"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip1")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .status()
        .unwrap_or_else(|e| panic!("failed to invoke cargo for {crate_dir}: {e}"));
    assert!(status.success(), "building {crate_dir} wasm failed");

    let wasm_path = target_dir
        .join("wasm32-wasip1")
        .join("release")
        .join(format!("{lib_name}.wasm"));
    assert!(
        wasm_path.exists(),
        "expected {crate_dir} wasm at {}",
        wasm_path.display()
    );
    println!("cargo:rustc-env={env_var}={}", wasm_path.display());
}
