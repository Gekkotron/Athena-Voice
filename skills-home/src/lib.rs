//! `skills-home` — home-automation MQTT skill for Athena-Voice.
//!
//! Guest ABI (matches `crates/athena-voice-runtime/src/wasm/registry.rs`):
//! - `pattern_rules(locale) -> String`  (JSON of `Vec<PatternRule>`)
//! - `handle(intent_json) -> String`    (JSON of `Result<SkillResponse, SkillError>`)
//!
//! Entities are declared in the per-skill config as a *single* JSON-encoded
//! string under the `entities` key (because per-skill `config` is a
//! `HashMap<String, String>`). See `Entity` below for the schema.

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::strsim::normalized_damerau_levenshtein;
use athena_voice_skill_sdk::{
    Intent, PatternRule, Skill, SkillError, SkillResponse, SlotKind, SlotSpec,
};
use extism_pdk::{FnResult, plugin_fn};
use once_cell::sync::OnceCell;
use serde::Deserialize;

/// Minimum similarity for a fuzzy device-name match to be accepted.
const FUZZY_THRESHOLD: f64 = 0.75;

#[derive(Debug, Clone, Deserialize)]
struct Entity {
    name: String,
    room: String,
    kind: String,
    set_topic: String,
    on_payload: String,
    off_payload: String,
}

static ENTITIES: OnceCell<Vec<Entity>> = OnceCell::new();

fn entities(ctx: &HostCtx) -> &'static [Entity] {
    ENTITIES
        .get_or_init(|| {
            let raw = ctx.config_get_toml("entities").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            match serde_json::from_str::<Vec<Entity>>(&raw) {
                Ok(v) => v,
                Err(e) => {
                    ctx.log("warn", &format!("home: failed to parse entities: {e}"));
                    Vec::new()
                }
            }
        })
        .as_slice()
}

struct HomeSkill;

impl Skill for HomeSkill {
    fn name(&self) -> &str {
        "home"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        if locale != "fr" {
            return Vec::new();
        }
        let room = vec![SlotSpec {
            name: "room".into(),
            kind: SlotKind::String,
        }];
        let device = vec![SlotSpec {
            name: "device".into(),
            kind: SlotKind::String,
        }];
        vec![
            PatternRule {
                intent: "home.light.on".into(),
                phrases: vec!["allume la lumière du {room}".into()],
                slots: room.clone(),
            },
            PatternRule {
                intent: "home.light.off".into(),
                phrases: vec!["éteins la lumière du {room}".into()],
                slots: room,
            },
            PatternRule {
                intent: "home.device.on".into(),
                phrases: vec!["allume {device}".into()],
                slots: device.clone(),
            },
            PatternRule {
                intent: "home.device.off".into(),
                phrases: vec!["éteins {device}".into()],
                slots: device,
            },
        ]
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let list = entities(ctx);

        match intent.name.as_str() {
            "home.light.on" | "home.light.off" => {
                let room = slot_str(&intent, "room");
                let on = intent.name == "home.light.on";
                match find_light(list, &room) {
                    Some(entity) => publish_command(ctx, entity, on),
                    None => Ok(SkillResponse::speak(format!(
                        "désolé, je ne connais pas la lumière du {room}"
                    ))),
                }
            }
            "home.device.on" | "home.device.off" => {
                let device = slot_str(&intent, "device");
                let on = intent.name == "home.device.on";
                match find_device_fuzzy(ctx, list, &device) {
                    Some(entity) => publish_command(ctx, entity, on),
                    None => Ok(SkillResponse::speak(format!(
                        "désolé, je ne connais pas {device}"
                    ))),
                }
            }
            other => Err(SkillError::Custom(format!("unknown intent: {other}"))),
        }
    }
}

fn slot_str(intent: &Intent, key: &str) -> String {
    intent
        .slots
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn find_light<'a>(entities: &'a [Entity], room: &str) -> Option<&'a Entity> {
    entities
        .iter()
        .find(|e| e.kind == "light" && e.room.eq_ignore_ascii_case(room))
}

fn find_device_fuzzy<'a>(ctx: &HostCtx, entities: &'a [Entity], query: &str) -> Option<&'a Entity> {
    if query.is_empty() || entities.is_empty() {
        return None;
    }
    let query_lc = query.to_lowercase();
    let mut best: Option<(&'a Entity, f64)> = None;
    for e in entities {
        let sim = normalized_damerau_levenshtein(&e.name.to_lowercase(), &query_lc);
        ctx.log(
            "debug",
            &format!("home: candidate '{}' similarity {:.3}", e.name, sim),
        );
        if sim >= FUZZY_THRESHOLD && best.map(|(_, s)| sim > s).unwrap_or(true) {
            best = Some((e, sim));
        }
    }
    best.map(|(e, _)| e)
}

fn publish_command(
    ctx: &HostCtx,
    entity: &Entity,
    on: bool,
) -> Result<SkillResponse, SkillError> {
    let payload = if on {
        entity.on_payload.as_bytes()
    } else {
        entity.off_payload.as_bytes()
    };
    match ctx.mqtt_publish(&entity.set_topic, payload) {
        Ok(()) => Ok(SkillResponse::speak("d'accord")),
        Err(e) => {
            ctx.log(
                "warn",
                &format!("home: publish {} failed: {e}", entity.set_topic),
            );
            Ok(SkillResponse::speak("je n'ai pas pu envoyer la commande"))
        }
    }
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = HomeSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut skill = HomeSkill;
    let mut c = HostCtx::for_testing();
    let result = skill.handle(intent, &mut c);
    Ok(serde_json::to_string(&result)?)
}
