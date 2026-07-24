//! Time intent: speaks the host's local wall-clock time.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, SkillError, SkillResponse};

pub fn patterns(locale: &str) -> Vec<PatternRule> {
    let phrases: Vec<String> = match locale {
        "fr" => vec!["quelle heure est-il".into(), "il est quelle heure".into()],
        "en" => vec!["what time is it".into(), "what's the time".into()],
        _ => return Vec::new(),
    };
    vec![PatternRule {
        intent: "time.query".into(),
        phrases,
        slots: Vec::new(),
    }]
}

pub fn handle(intent: &Intent, ctx: &HostCtx) -> Result<SkillResponse, SkillError> {
    let t = ctx.local_time()?;
    let speech = if intent.locale.starts_with("en") {
        speak_time_en(t.hour(), t.minute())
    } else {
        speak_time_fr(t.hour(), t.minute())
    };
    Ok(SkillResponse::speak(speech))
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

fn speak_time_en(hour: u8, minute: u8) -> String {
    let (h12, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    if minute == 0 {
        format!("it is {h12} {suffix}")
    } else {
        format!("it is {h12}:{minute:02} {suffix}")
    }
}
