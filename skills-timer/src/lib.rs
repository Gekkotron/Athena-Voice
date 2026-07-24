//! `skills-timer` — a timer / reminder WASM skill for Athena-Voice.
//!
//! Guest ABI (matches `crates/athena-voice-runtime/src/wasm/registry.rs`):
//! - `pattern_rules(locale) -> String`   (JSON of `Vec<PatternRule>`)
//! - `handle(intent_json) -> String`     (JSON of `Result<SkillResponse, SkillError>`)

mod duration;

use std::time::{SystemTime, UNIX_EPOCH};

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{
    ConfigSchema, Intent, PatternRule, Skill, SkillError, SkillResponse, SlotKind, SlotSpec,
};
use extism_pdk::{FnResult, plugin_fn};

use duration::{parse_en_duration, parse_fr_duration};

const MAX_SECONDS: u64 = 24 * 3600;

struct TimerSkill;

impl Skill for TimerSkill {
    fn name(&self) -> &str {
        "timer"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        let phrases: Vec<String> = match locale {
            "fr" => vec![
                "mets un minuteur de {duration}".into(),
                "minuteur {duration}".into(),
                "réveille-moi dans {duration}".into(),
            ],
            "en" => vec![
                "set a timer for {duration}".into(),
                "timer for {duration}".into(),
                "wake me up in {duration}".into(),
            ],
            _ => return Vec::new(),
        };
        let slots = vec![SlotSpec {
            name: "duration".into(),
            kind: SlotKind::String,
        }];
        vec![PatternRule {
            intent: "timer.set".into(),
            phrases,
            slots,
        }]
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let duration_text = intent
            .slots
            .get("duration")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let en = intent.locale.starts_with("en");
        let parsed = if en {
            parse_en_duration(&duration_text)
        } else {
            parse_fr_duration(&duration_text)
        };
        let Some(seconds) = parsed else {
            return Ok(SkillResponse::speak(if en {
                "sorry, I didn't understand the timer duration"
            } else {
                "désolé, je n'ai pas compris la durée du minuteur"
            }));
        };

        if seconds > MAX_SECONDS {
            return Ok(SkillResponse::speak(if en {
                "sorry, I only handle timers under twenty-four hours"
            } else {
                "désolé, je ne gère que les minuteurs de moins de vingt-quatre heures"
            }));
        }

        let now_ms = now_millis();
        let fires_at_ms = now_ms + (seconds as i64) * 1000;

        let payload = serde_json::to_vec(&serde_json::json!({ "seconds": seconds }))
            .map_err(|e| SkillError::Custom(format!("payload encode: {e}")))?;

    let id = ctx.schedule_mqtt(fires_at_ms, "athena/skills/timer/expired", &payload)?;
    ctx.tmp_set(&format!("timer/{id}"), &seconds.to_le_bytes(), seconds)?;
    // Also set persistent kv for cross-restart durability
    ctx.state_set(&format!("timer/{id}"), &seconds.to_le_bytes())?;

        Ok(SkillResponse::speak(if en {
            format!("okay, timer for {duration_text} started")
        } else {
            format!("d'accord, minuteur de {duration_text} lancé")
        }))
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn ctx() -> HostCtx {
    HostCtx::for_testing()
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = TimerSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut skill = TimerSkill;
    let mut c = ctx();
    let result = skill.handle(intent, &mut c);
    Ok(serde_json::to_string(&result)?)
}

#[plugin_fn]
pub fn config_schema(_input: String) -> FnResult<String> {
    let schema = ConfigSchema { fields: vec![] };
    Ok(serde_json::to_string(&schema)?)
}
