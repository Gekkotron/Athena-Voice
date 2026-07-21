//! Test patterns for skill-driven audio playback.
//!
//! Exposes "joue un son" → `audio.play`.

use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse};

pub fn patterns() -> Vec<PatternRule> {
    vec![PatternRule::new(
        "audio.play",
        vec!["joue un son"],
        "fr".into(),
    )]
}

pub async fn handle(_intent: Intent) -> Result<SkillResponse, SkillError> {
    // 8kHz, 1ch, 1.0 sine wave for 50ms
    let sample_rate = 8000;
    let duration_sec = 0.05;
    let num_samples = (sample_rate as f32 * duration_sec) as usize;
    let samples: Vec<f32> = (0..num_samples)
        .map(|i| ((2.0 * std::f32::consts::PI * 440.0 * i as f32) / sample_rate as f32).sin())
        .collect();
    Ok(SkillResponse::sampled_pcm(sample_rate, samples))
}