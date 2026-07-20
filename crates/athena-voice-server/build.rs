//! Symlink real models into OUT_DIR.

use std::os::unix::fs::symlink;
use std::path::Path;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let model_dir = Path::new(&out_dir).join("models");
    std::fs::create_dir_all(&model_dir).unwrap();

    // Symlink real models.
    symlink_model("ggml-small-french-q5_1.bin");
    symlink_model("piper-fr.onnx");
    symlink_model("piper-fr.onnx.json");
    symlink_model("hotword.onnx");
}

fn symlink_model(filename: &str) {
    let src = Path::new("models").join(filename);
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("models").join(filename);

    if src.exists() && !dest.exists() {
        symlink(src, dest).unwrap();
        println!("cargo:warning=Symlinked {}", filename);
    }
}
