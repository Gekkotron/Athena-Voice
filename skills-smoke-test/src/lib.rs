//! `skills-smoke-test` — a WASM skill that exercises every host function on
//! the Athena-Voice skill ABI.

mod time;
mod audio;

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, Skill, SkillError, SkillResponse};
use extism_pdk::{FnResult, plugin_fn};

struct SmokeSkill;

impl Skill for SmokeSkill {
    fn name(&self) -> &str {
        "smoke-test"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        let mut rules = time::patterns(locale);
        if locale == "fr" {
            rules.extend(audio::patterns());
        }
        rules
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let name = intent.name.clone();
        let result = match name.as_str() {
            "time.query" => time::handle(&intent, ctx),
            "audio.play" | "audio.volume" => audio::handle(intent),
            _ => Err(SkillError::Custom(format!("unknown intent {name}"))),
        };
        if result.is_ok() {
            // Record the last handled intent so retention tests can observe
            // a state write with the automatic timestamp prefix.
            ctx.state_set("last_intent", name.as_bytes())?;
        }
        result
    }
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = SmokeSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut ctx = HostCtx::for_testing();
    let mut skill = SmokeSkill;
    let result = skill.handle(intent, &mut ctx);
    Ok(serde_json::to_string(&result)?)
}