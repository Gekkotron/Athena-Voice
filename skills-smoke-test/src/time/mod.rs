//! Test patterns for time-intent.
use athena_voice_skill_sdk::{PatternRule, SkillError, SkillResponse};

pub fn patterns() -> Vec<PatternRule> {
    vec![PatternRule::new(
        "time.query",
        vec!["quelle heure est-il"],
        "fr".into(),
    )]
}

pub async fn handle(_intent: athena_voice_skill_sdk::Intent) -> Result<SkillResponse, SkillError> {
    Ok(SkillResponse::speak("il est huit heure"))
}