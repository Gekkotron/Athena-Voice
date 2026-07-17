//! `skills-smoke-test` — a WASM skill that exercises every host function on
//! the Athena-Voice skill ABI. Used by the runtime's integration tests as a
//! live fixture proving the host ↔ guest bridge round-trips end-to-end.
//!
//! Guest ABI (matches `crates/athena-voice-runtime/src/wasm/registry.rs`):
//! - `pattern_rules(locale) -> String`   (JSON of `Vec<PatternRule>`)
//! - `handle(intent_json) -> String`     (JSON of `Result<SkillResponse, SkillError>`)

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{Intent, PatternRule, Skill, SkillError, SkillResponse};
use extism_pdk::{FnResult, plugin_fn};

struct SmokeSkill;

impl Skill for SmokeSkill {
    fn name(&self) -> &str {
        "smoke-test"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        if locale == "fr" {
            vec![PatternRule {
                intent: "time.query".into(),
                phrases: vec!["quelle heure est-il".into()],
                slots: vec![],
            }]
        } else {
            vec![]
        }
    }

    fn handle(&mut self, _intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        ctx.log("info", "smoke-test: handling time.query");

        let greeting = ctx.config_get("greeting").unwrap_or_default();
        ctx.log("debug", &format!("config greeting={greeting}"));

        ctx.state_set("last_intent", b"time.query")?;
        let _prev = ctx.state_get("last_intent")?;

        ctx.mqtt_publish("athena/skills/smoke-test/tick", b"pong")?;

        // The runtime supplies the allowed host via the per-skill
        // `http_allowlist`. The integration test points this at a mock server.
        let _ = ctx.http_get_json("http://smoke.local/ping");

        Ok(SkillResponse::speak("il est huit heure"))
    }
}

// The SDK's `HostCtx` is a unit type on the guest side; we just need one to
// pass into `Skill::handle`.
fn ctx() -> HostCtx {
    HostCtx::for_testing()
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = SmokeSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut skill = SmokeSkill;
    let mut c = ctx();
    let result = skill.handle(intent, &mut c);
    Ok(serde_json::to_string(&result)?)
}
