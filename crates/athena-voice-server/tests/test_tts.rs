//! Piper TTS loader contract.
//!
//! The files under `models/` are placeholders, not real ONNX graphs, so the
//! only contract testable today is that `PiperTts::load` fails cleanly on
//! invalid input instead of panicking. Once a real Piper voice is dropped
//! into `models/`, extend this with an actual synthesis test.

use std::path::Path;

use athena_voice_server::tts::PiperTts;

#[test]
fn load_rejects_placeholder_model_gracefully() {
    let models = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .join("models");
    let result = PiperTts::load(&models, "piper-fr", "fr");
    assert!(
        result.is_err(),
        "placeholder model bytes must not load as a valid ONNX graph"
    );
}

#[test]
fn load_missing_model_dir_errors() {
    let result = PiperTts::load(Path::new("/nonexistent/models"), "piper-fr", "fr");
    assert!(result.is_err());
}
