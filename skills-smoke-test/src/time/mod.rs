//! Time intent: speaks the host's local wall-clock time.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse};

pub fn patterns() -> Vec<PatternRule> {
    vec![PatternRule {
        intent: "time.query".into(),
        phrases: vec!["quelle heure est-il".into()],
        slots: Vec::new(),
    }]
}

pub fn handle(_intent: Intent, ctx: &HostCtx) -> Result<SkillResponse, SkillError> {
    let t = ctx.local_time()?;
    Ok(SkillResponse::speak(speak_time_fr(t.hour(), t.minute())))
}

fn speak_time_fr(hour: u8, minute: u8) -> String {
    let h = match hour {
        0 => "minuit".to_string(),
        12 => "midi".to_string(),
        h => format!("{h} heures"),
    };
    if minute == 0 {
        format!("il est {h}")
    } else {
        format!("il est {h} {minute}")
    }
}
