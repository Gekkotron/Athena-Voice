//! Test skill for INI config.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{PatternRule, Skill, SkillError, SkillResponse};
use extism_pdk::{FnResult, plugin_fn};

struct AudioTestSkill;

impl Skill for AudioTestSkill {
    fn name(&self) -> &str {
        "audio-test"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        vec![PatternRule::new(
            "audio.test",
            vec!["test config"],
            locale.to_string(),
        )]
    }

    fn handle(&mut self, _intent: athena_voice_skill_sdk::Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        if let Some(ini) = ctx.config_get()? {
            let speed = ini.section("audio").and_then(|s| s.get("speed"));
            return Ok(SkillResponse::speak(format!("Speed: {:?}", speed)));
        }
        Ok(SkillResponse::speak("No config"))
    }
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = AudioTestSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: athena_voice_skill_sdk::Intent = serde_json::from_str(&intent_json)?;
    let mut ctx = HostCtx::for_testing();
    let result = AudioTestSkill.handle(intent, &mut ctx);
    Ok(serde_json::to_string(&result)?)
}