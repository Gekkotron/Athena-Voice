//! Test patterns for skill-driven audio playback.
use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse};

use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse, SlotKind};

pub fn patterns() -> Vec<PatternRule> {
    vec![
        PatternRule::new(
            "audio.play",
            vec!["joue un son"],
            "fr".into(),
        ),
        PatternRule::new(
            "audio.volume",
            vec!["mets le volume à {}%"],
            "fr".into(),
        ).with_slot("percent", SlotKind::Number),
    ]
}

pub async fn handle(intent: Intent) -> Result<SkillResponse, SkillError> {
    match intent.name.as_str() {
        "audio.play" => {
            // 8kHz, 1ch, 1.0 sine wave for 50ms
            let sample_rate = 8000;
            let duration_sec = 0.05;
            let num_samples = (sample_rate as f32 * duration_sec) as usize;
            let samples: Vec<f32> = (0..num_samples)
                .map(|i| ((2.0 * std::f32::consts::PI * 440.0 * i as f32) / sample_rate as f32).sin())
                .collect();
            Ok(SkillResponse::sampled_pcm(sample_rate, samples))
        }
        "audio.volume" => {
            let percent = intent.slots.get("percent").and_then(|s| s.as_float()).unwrap_or(100.0);
            let volume = (percent / 100.0).clamp(0.0, 1.5);
            Ok(SkillResponse::volume(volume))
        }
        _ => Err(SkillError::Custom(format!("unknown audio intent {}", intent.name))),
    }
}