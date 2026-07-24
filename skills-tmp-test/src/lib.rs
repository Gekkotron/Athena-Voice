//! Test skill for temporary storage.
use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{
    Intent, PatternRule, Skill, SkillError, SkillResponse, SlotKind, SlotSpec,
};
use extism_pdk::{FnResult, plugin_fn};

struct TmpTestSkill;

impl Skill for TmpTestSkill {
    fn name(&self) -> &str {
        "tmp-test"
    }

    fn pattern_rules(&self, _locale: &str) -> Vec<PatternRule> {
        vec![PatternRule {
            intent: "tmp.test".into(),
            phrases: vec!["test tmp {key} {value}".into()],
            slots: vec![
                SlotSpec {
                    name: "key".into(),
                    kind: SlotKind::String,
                },
                SlotSpec {
                    name: "value".into(),
                    kind: SlotKind::String,
                },
            ],
        }]
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let key = intent
            .slots
            .get("key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let val = intent
            .slots
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        // Test: set → get → verify (2s TTL).
        ctx.tmp_set(&key, val.as_bytes(), 2)?;
        let stored = ctx.tmp_get(&key)?;

        Ok(if stored.as_deref() == Some(val.as_bytes()) {
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
