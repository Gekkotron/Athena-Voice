//! Test skill for INI config.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, Skill, SkillError, SkillResponse};
use extism_pdk::{FnResult, plugin_fn};

struct AudioTestSkill;

impl Skill for AudioTestSkill {
    fn name(&self) -> &str {
        "audio-test"
    }

    fn pattern_rules(&self, _locale: &str) -> Vec<PatternRule> {
        vec![PatternRule {
            intent: "audio.test".into(),
            phrases: vec!["test config".into()],
            slots: Vec::new(),
        }]
    }

    fn handle(
        &mut self,
        _intent: Intent,
        ctx: &mut HostCtx,
    ) -> Result<SkillResponse, SkillError> {
        if let Some(ini) = ctx.config_get()? {
            let speed = ini
                .section("audio")
                .and_then(|s| s.get("speed").cloned().flatten());
            return Ok(SkillResponse::speak(format!("Speed: {speed:?}")));
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
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut ctx = HostCtx::for_testing();
    let result = AudioTestSkill.handle(intent, &mut ctx);
    Ok(serde_json::to_string(&result)?)
}
