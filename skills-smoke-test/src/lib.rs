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
        let mut rules = time::patterns();
        rules.extend(audio::patterns());
        rules
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        match intent.name.as_str() {
            "time.query" => time::handle(intent),
            "audio.play" => audio::handle(intent),
            _ => Err(SkillError::Custom(format!("unknown intent {}", intent.name))),
        }
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