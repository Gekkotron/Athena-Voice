//! Test skill for temporary storage.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, Skill, SkillError, SkillResponse};
use extism_pdk::{FnResult, plugin_fn};

struct TmpTestSkill;

impl Skill for TmpTestSkill {
    fn name(&self) -> &str {
        "tmp-test"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        vec![PatternRule::new(
            "tmp.test",
            vec!["test tmp {key} {value}"],
            locale.to_string(),
        )
        .with_slot("key", athena_voice_skill_sdk::SlotKind::String)
        .with_slot("value", athena_voice_skill_sdk::SlotKind::String)]
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let key = intent.slots.get("key").and_then(|s| s.as_string()).unwrap_or_default();
        let val = intent.slots.get("value").and_then(|s| s.as_string()).unwrap_or_default();
        
        // Test: set → get → verify → expire
        ctx.tmp_set(&key, val.as_bytes(), 2)?; // Expire in 2 sec
        let stored = ctx.tmp_get(&key)?;
        
        Ok(if stored == Some(val.as_bytes().to_vec()) {
            SkillResponse::speak("found")
        } else {
            SkillResponse::speak("not found")
        })
    }
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = TmpTestSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut ctx = HostCtx::for_testing();
    let result = TmpTestSkill.handle(intent, &mut ctx);
    Ok(serde_json::to_string(&result)?)
}