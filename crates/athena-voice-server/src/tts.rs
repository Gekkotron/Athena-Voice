//! Piper TTS via ONNX.

use std::path::Path;

use tract_onnx::prelude::*;

/// Piper TTS model.
#[derive(Debug)]
pub struct PiperTts {
    model: RunnableModel<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>,
    // The remaining fields feed the real Piper tokenizer, which is still a
    // TODO in `tokenize`.
    #[allow(dead_code)]
    voice: String,
    #[allow(dead_code)]
    sample_rate: u32,
    #[allow(dead_code)]
    tokenizer: serde_json::Value,
}

impl PiperTts {
    /// Load the model.
    pub fn load(model_dir: &Path, model_name: &str, voice: &str) -> anyhow::Result<Self> {
        let model_path = model_dir.join(model_name).with_extension("onnx");
        let json_path = model_dir.join(model_name).with_extension("onnx.json");

        // Load ONNX model.
        let model = onnx()
            .model_for_path(model_path)?
            .into_typed()?
            .into_optimized()?
            .into_runnable()?;

        // Load tokenizer config.
        let tokenizer: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(json_path)?)?;
        let sample_rate = tokenizer["audio"]
            .get("sample_rate")
            .and_then(|v| v.as_u64())
            .unwrap_or(22050) as u32;

        Ok(Self {
            model,
            voice: voice.to_string(),
            sample_rate,
            tokenizer,
        })
    }

    /// Synthesize speech.
    pub fn synthesize(&self, text: &str) -> anyhow::Result<Vec<i16>> {
        // Tokenize text.
        let tokens = self.tokenize(text)?;
        let input = tract_ndarray::Array2::from_shape_vec((1, tokens.len()), tokens)?.into_tensor();

        // Run inference.
        let result = self.model.run(tvec![input.into()])?;
        let output = result[0].to_array_view::<f32>()?;

        // Convert to i16 PCM.
        Ok(output
            .iter()
            .map(|&x| (x.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect())
    }

    /// Tokenize text using Piper's tokenizer.
    fn tokenize(&self, text: &str) -> anyhow::Result<Vec<i64>> {
        let mut tokens = Vec::new();
        for c in text.chars() {
            // Piper uses a custom vocabulary.
            // TODO: Replace with real Piper tokenizer.
            tokens.push(c as i64);
        }
        Ok(tokens)
    }
}
