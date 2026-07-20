use std::path::Path;

use athena_voice_server::PiperTts;

#[test]
fn test_tts_inference() {
    let model_dir = Path::new("../../models");
    let tts = PiperTts::load(model_dir, "piper-fr", "bl_lightspeed").unwrap();
    let pcm = tts.synthesize("Bonjour").unwrap();
    assert!(!pcm.is_empty());
    // Play via `cpal` or save to WAV.
}
