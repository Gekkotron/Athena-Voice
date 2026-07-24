//! `skills-jeedom` — Jeedom sensor connector for Athena-Voice.
//!
//! Reads sensor command values from a Jeedom box over its HTTP API
//! (`/core/api/jeeApi.php?apikey=…&type=cmd&id=…`, which returns the raw
//! value — valid JSON for numeric sensors).
//!
//! Per-skill config (`[skills.jeedom]`):
//! - `http_allowlist` must contain the Jeedom host.
//! - `config = { base_url = "http://jeedom.local", api_key = "…", sensors = "…" }`
//!   where `sensors` is a JSON-encoded array (the per-skill config map is
//!   `HashMap<String, String>`):
//!   `[{"name":"température du salon","id":123,"unit":"degrés"}, …]`
//!   `unit` is spoken after the value and may be empty.
//!
//! The API key only travels on your LAN, but treat it like a password:
//! prefer a Jeedom API key restricted to the sensors you expose.

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::strsim::normalized_damerau_levenshtein;
use athena_voice_skill_sdk::{
    ConfigField, ConfigSchema, FieldKind, Intent, ItemField, PatternRule, Skill, SkillError,
    SkillResponse, SlotKind, SlotSpec,
};
use extism_pdk::{FnResult, plugin_fn};
use once_cell::sync::OnceCell;
use serde::Deserialize;

/// Minimum similarity for a fuzzy sensor-name match to be accepted.
const FUZZY_THRESHOLD: f64 = 0.75;

#[derive(Debug, Clone, Deserialize)]
struct Sensor {
    name: String,
    id: u64,
    #[serde(default)]
    unit: String,
}

static SENSORS: OnceCell<Vec<Sensor>> = OnceCell::new();

fn sensors(ctx: &HostCtx) -> &'static [Sensor] {
    SENSORS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("sensors").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            match serde_json::from_str::<Vec<Sensor>>(&raw) {
                Ok(v) => v,
                Err(e) => {
                    ctx.log("warn", &format!("jeedom: failed to parse sensors: {e}"));
                    Vec::new()
                }
            }
        })
        .as_slice()
}

struct JeedomSkill;

impl Skill for JeedomSkill {
    fn name(&self) -> &str {
        "jeedom"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        // No sensors configured → no patterns: installs without a
        // [skills.jeedom] config must not capture "donne-moi la …"
        // utterances just to apologise about unknown sensors.
        let configured = sensors(&HostCtx::for_testing());
        if configured.is_empty() {
            return Vec::new();
        }
        let mut phrases: Vec<String> = match locale {
            "fr" => vec![
                "capteur {sensor}".into(),
                "valeur du capteur {sensor}".into(),
                "donne-moi la {sensor}".into(),
                "combien fait la {sensor}".into(),
            ],
            "en" => vec![
                "sensor {sensor}".into(),
                "sensor value {sensor}".into(),
                "give me the {sensor}".into(),
                "read the {sensor}".into(),
            ],
            _ => return Vec::new(),
        };
        let mut rules = vec![PatternRule {
            intent: "jeedom.read".into(),
            phrases: std::mem::take(&mut phrases),
            slots: vec![SlotSpec {
                name: "sensor".into(),
                kind: SlotKind::String,
            }],
        }];
        // Each configured sensor also contributes literal phrasings built
        // from its own name — "quelle est la température du salon" must
        // route here rather than to the weather skill's generic
        // "quelle est la température" (the matcher prefers the more
        // specific phrase on near-ties, which also absorbs STT slips).
        // The sensor id rides in the intent name since literals have no slot.
        for sensor in configured {
            let name = &sensor.name;
            let literal_phrases: Vec<String> = match locale {
                "fr" => vec![
                    format!("quelle est la {name}"),
                    format!("quel est le niveau de {name}"),
                ],
                "en" => vec![
                    format!("what is the {name}"),
                    format!("what's the {name}"),
                ],
                _ => Vec::new(),
            };
            rules.push(PatternRule {
                intent: format!("jeedom.read.{}", sensor.id),
                phrases: literal_phrases,
                slots: Vec::new(),
            });
        }
        rules
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let en = intent.locale.starts_with("en");

        // Per-sensor literal rules carry the id in the intent name.
        if let Some(id) = intent
            .name
            .strip_prefix("jeedom.read.")
            .and_then(|id| id.parse::<u64>().ok())
        {
            if let Some(sensor) = sensors(ctx).iter().find(|s| s.id == id) {
                return speak_reading(ctx, sensor, en);
            }
        }

        let asked = intent
            .slots
            .get("sensor")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if asked.is_empty() {
            return Ok(SkillResponse::speak(if en {
                "which sensor should I read?".to_string()
            } else {
                "quel capteur dois-je lire ?".to_string()
            }));
        }

        let Some(sensor) = resolve_sensor(ctx, &asked) else {
            return Ok(SkillResponse::speak(if en {
                format!("sorry, I don't know a sensor called {asked}")
            } else {
                format!("désolé, je ne connais pas de capteur {asked}")
            }));
        };

        speak_reading(ctx, sensor, en)
    }
}

/// Reads the sensor and phrases the answer in the session's language.
fn speak_reading(ctx: &HostCtx, sensor: &Sensor, en: bool) -> Result<SkillResponse, SkillError> {
    match read_value(ctx, sensor) {
        Ok(value) => {
            let unit = if sensor.unit.is_empty() {
                String::new()
            } else {
                format!(" {}", sensor.unit)
            };
            Ok(SkillResponse::speak(if en {
                format!("the {} is {value}{unit}", sensor.name)
            } else {
                format!("la {} est de {value}{unit}", sensor.name)
            }))
        }
        Err(()) => Ok(SkillResponse::speak(if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        })),
    }
}

/// Fuzzy-resolves the spoken sensor name against the configured list;
/// highest similarity above the threshold wins.
fn resolve_sensor<'a>(ctx: &HostCtx, asked: &str) -> Option<&'a Sensor> {
    let asked_lower = asked.to_lowercase();
    sensors(ctx)
        .iter()
        .map(|s| {
            (
                normalized_damerau_levenshtein(&s.name.to_lowercase(), &asked_lower),
                s,
            )
        })
        .filter(|(sim, _)| *sim >= FUZZY_THRESHOLD)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
}

/// Reads one command value through Jeedom's simple HTTP GET API.
fn read_value(ctx: &HostCtx, sensor: &Sensor) -> Result<String, ()> {
    let base = ctx
        .config_get_toml("base_url")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    let api_key = ctx
        .config_get_toml("api_key")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    let url = format!(
        "{}/core/api/jeeApi.php?apikey={api_key}&type=cmd&id={}",
        base.trim_end_matches('/'),
        sensor.id
    );
    match ctx.http_get_json(&url) {
        // Numeric sensors come back as a bare JSON scalar; be tolerant of
        // string-wrapped numbers and `{"value": …}` envelopes too.
        Ok(v) => {
            let value = v
                .get("value")
                .cloned()
                .unwrap_or(v);
            let spoken = match value {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            Ok(spoken)
        }
        Err(e) => {
            ctx.log("warn", &format!("jeedom: HTTP failed: {e}"));
            Err(())
        }
    }
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = JeedomSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut ctx = HostCtx::for_testing();
    let result = JeedomSkill.handle(intent, &mut ctx);
    Ok(serde_json::to_string(&result)?)
}

#[plugin_fn]
pub fn config_schema(_input: String) -> FnResult<String> {
    let schema = ConfigSchema {
        fields: vec![
            ConfigField {
                key: "base_url".into(),
                label: "Jeedom URL".into(),
                kind: FieldKind::Url,
                required: true,
                help: "e.g. http://192.168.1.91 — the host is allowed for HTTP automatically"
                    .into(),
                default: String::new(),
                item_fields: vec![],
            },
            ConfigField {
                key: "api_key".into(),
                label: "API key".into(),
                kind: FieldKind::Secret,
                required: true,
                help: "Jeedom → Settings → System → Configuration → API".into(),
                default: String::new(),
                item_fields: vec![],
            },
            ConfigField {
                key: "sensors".into(),
                label: "Sensors".into(),
                kind: FieldKind::List,
                required: false,
                help: "Spoken name → Jeedom command id".into(),
                default: String::new(),
                item_fields: vec![
                    ItemField {
                        key: "name".into(),
                        kind: FieldKind::String,
                    },
                    ItemField {
                        key: "id".into(),
                        kind: FieldKind::Number,
                    },
                    ItemField {
                        key: "unit".into(),
                        kind: FieldKind::String,
                    },
                ],
            },
        ],
    };
    Ok(serde_json::to_string(&schema)?)
}
