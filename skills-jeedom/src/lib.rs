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
//!   `[{"name":"température du salon","id":123,"unit":"degrés","room":"salon"}, …]`
//!   `unit` is spoken after the value and may be empty. `room` (optional)
//!   enables room-scoped phrasings ("température dans le salon") and
//!   groups sensors for enumeration ("toutes les températures"). `kind`
//!   (optional) set to `"binary"` switches reading phrasing to the spoken
//!   `on_label`/`off_label` (e.g. "ouverte"/"fermée") instead of a numeric
//!   value; anything else, including the default empty string, is numeric.
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
    /// Room the sensor lives in, e.g. "salon". Empty when not discovered /
    /// not filled in by the user; enables room-scoped phrasings.
    #[serde(default)]
    room: String,
    /// `"binary"` switches reading phrasing to on/off labels; anything else
    /// (including empty, the default) is treated as numeric.
    #[serde(default)]
    kind: String,
    /// Spoken word for the "on"/true state of a binary sensor.
    #[serde(default)]
    on_label: String,
    /// Spoken word for the "off"/false state of a binary sensor.
    #[serde(default)]
    off_label: String,
    /// French connector spoken before the room (« du », « de la », « d' »).
    /// Filled by discovery's article guess; empty keeps the legacy
    /// both-genders phrase enumeration.
    #[serde(default)]
    prefix: String,
}

static SENSORS: OnceCell<Vec<Sensor>> = OnceCell::new();

/// Strips symbol characters (emoji, icons) from a spoken/matchable field.
/// Jeedom discovery can compose names and rooms carrying icons
/// ("salon 🖴"); STT never hears them and TTS should never read them.
/// Letters/digits of any script, apostrophes, and hyphens survive; dropped
/// characters collapse into single spaces.
fn clean_spoken(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '-' {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            out.push(c);
            pending_space = false;
        } else {
            pending_space = true;
        }
    }
    out
}

/// Parses the configured sensor list, sanitizing every field that ends up
/// in generated patterns or spoken answers. `unit` is deliberately NOT
/// cleaned: "°C" must survive for TTS to read it as degrees.
fn parse_sensors(raw: &str) -> Vec<Sensor> {
    let Ok(mut v) = serde_json::from_str::<Vec<Sensor>>(raw) else {
        return Vec::new();
    };
    for s in &mut v {
        s.name = clean_spoken(&s.name);
        s.room = clean_spoken(&s.room);
        s.on_label = clean_spoken(&s.on_label);
        s.off_label = clean_spoken(&s.off_label);
        s.prefix = clean_spoken(&s.prefix.replace('’', "'"));
    }
    v
}

fn sensors(ctx: &HostCtx) -> &'static [Sensor] {
    SENSORS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("sensors").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            let v = parse_sensors(&raw);
            if v.is_empty() {
                ctx.log("warn", "jeedom: sensor list empty or failed to parse");
            }
            v
        })
        .as_slice()
}

struct JeedomSkill;

impl Skill for JeedomSkill {
    fn name(&self) -> &str {
        "jeedom"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        rules_for(locale, sensors(&HostCtx::for_testing()))
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

        // Enumeration intents ask for every sensor sharing a metric
        // ("toutes les températures") rather than a single sensor.
        if let Some(metric) = intent.name.strip_prefix("jeedom.read_all.") {
            let list = sensors(ctx);
            let matching: Vec<&Sensor> =
                list.iter().filter(|s| metric_of(s) == metric || s.name.contains(metric)).collect();
            if matching.is_empty() {
                return Ok(SkillResponse::speak(if en {
                    format!("no sensor matches {metric}")
                } else {
                    format!("aucun capteur ne correspond à {metric}")
                }));
            }
            let mut clauses = Vec::new();
            for sensor in matching {
                match read_value(ctx, sensor) {
                    Ok(v) => clauses.push(enum_clause(sensor, &v, en)),
                    Err(()) => {
                        let place = if sensor.room.is_empty() { sensor.name.clone() } else { sensor.room.clone() };
                        clauses.push(if en { format!("{place} unavailable") } else { format!("{place} indisponible") });
                    }
                }
            }
            return Ok(SkillResponse::speak(clauses.join(", ")));
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

        let Some(sensor) = resolve_in(sensors(ctx), &asked) else {
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
        Ok(value) => Ok(SkillResponse::speak(phrase_reading(sensor, &value, en))),
        Err(()) => Ok(SkillResponse::speak(if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        })),
    }
}

/// Selects the spoken label for a binary sensor's raw value: the sensor's
/// own `on_label`/`off_label` when set, falling back to a generic
/// activé/désactivé (or on/off in English). Shared by `phrase_reading` and
/// `enum_clause` so both phrasings agree on what a binary reading says.
fn binary_label<'a>(sensor: &'a Sensor, value: &str, en: bool) -> &'a str {
    let on = value == "1" || value.eq_ignore_ascii_case("true");
    if on {
        if sensor.on_label.is_empty() { if en { "on" } else { "activé" } } else { &sensor.on_label }
    } else if sensor.off_label.is_empty() {
        if en { "off" } else { "désactivé" }
    } else {
        &sensor.off_label
    }
}

/// Phrases a raw reading in the session's language. Binary sensors speak
/// their configured on/off label (falling back to a generic activé/désactivé
/// or on/off); numeric sensors speak the value with its unit.
fn phrase_reading(sensor: &Sensor, value: &str, en: bool) -> String {
    if sensor.kind == "binary" {
        let label = binary_label(sensor, value, en);
        return if en {
            format!("the {} is {label}", sensor.name)
        } else {
            format!("la {} est {label}", sensor.name)
        };
    }
    let unit = if sensor.unit.is_empty() { String::new() } else { format!(" {}", sensor.unit) };
    if en {
        format!("the {} is {value}{unit}", sensor.name)
    } else {
        format!("la {} est de {value}{unit}", sensor.name)
    }
}

/// Builds one enumeration clause ("garage ouverte", "salon 21.5 degrés") for
/// a sensor's raw reading. Uses the sensor's room as the spoken place when
/// set (falling back to its full name), and the same binary label selection
/// as `phrase_reading` so enumeration and single-sensor readings never
/// disagree on what a binary value means.
fn enum_clause(sensor: &Sensor, value: &str, en: bool) -> String {
    let place = if sensor.room.is_empty() { sensor.name.clone() } else { sensor.room.clone() };
    if sensor.kind == "binary" {
        format!("{place} {}", binary_label(sensor, value, en))
    } else {
        let unit = if sensor.unit.is_empty() { String::new() } else { format!(" {}", sensor.unit) };
        format!("{place} {value}{unit}")
    }
}

/// The sensor's "metric word": its name with a trailing room reference
/// stripped, e.g. "température du salon" (room "salon") → "température".
/// Falls back to the full name when there's no room suffix to strip.
fn metric_of(sensor: &Sensor) -> String {
    let name = sensor.name.trim().to_lowercase();
    let room = sensor.room.trim().to_lowercase();
    if room.is_empty() || !name.ends_with(&room) {
        return name;
    }
    let head = name[..name.len() - room.len()].trim_end();
    let head = ["du", "de la", "de l’", "de l'", "de", "dans le", "dans la"]
        .iter()
        .find_map(|art| head.strip_suffix(art))
        .unwrap_or(head)
        .trim_end();
    if head.is_empty() { name } else { head.to_string() }
}

/// Fuzzy-resolves the spoken sensor name against a sensor list; highest
/// similarity above the threshold wins.
fn resolve_in<'a>(list: &'a [Sensor], asked: &str) -> Option<&'a Sensor> {
    let asked_lower = asked.to_lowercase();
    list.iter()
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

/// Builds the full set of match rules for a locale from a sensor list.
/// No sensors configured → no patterns: installs without a
/// `[skills.jeedom]` config must not capture "donne-moi la …" utterances
/// just to apologise about unknown sensors.
fn rules_for(locale: &str, configured: &[Sensor]) -> Vec<PatternRule> {
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
        let mut literal_phrases: Vec<String> = match locale {
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
        // Sensors tied to a room also answer room-scoped phrasings —
        // "quelle température dans le salon" — routed to the same literal
        // per-sensor intent. Both genders are offered since the matcher's
        // fuzzy tolerance benefits from covering "le"/"la" alike.
        if !sensor.room.is_empty() {
            let metric = metric_of(sensor);
            let room = &sensor.room;
            match locale {
                "fr" => literal_phrases.extend([
                    format!("quelle {metric} dans le {room}"),
                    format!("quelle {metric} dans la {room}"),
                    format!("{metric} dans le {room}"),
                    format!("{metric} dans la {room}"),
                    // Natural full-sentence forms: "quelle
                    // est la température dans le salon" / "… du salon".
                    format!("quelle est la {metric} dans le {room}"),
                    format!("quelle est la {metric} dans la {room}"),
                    format!("quelle est la {metric} du {room}"),
                    format!("quelle est la {metric} de la {room}"),
                    format!("{metric} du {room}"),
                    format!("{metric} de la {room}"),
                ]),
                "en" => literal_phrases.extend([
                    format!("{metric} in the {room}"),
                    format!("what is the {metric} in the {room}"),
                ]),
                _ => {}
            }
        }
        rules.push(PatternRule {
            intent: format!("jeedom.read.{}", sensor.id),
            phrases: literal_phrases,
            slots: Vec::new(),
        });
    }
    // One enumeration rule per distinct metric — "toutes les températures"
    // reads every sensor sharing that metric word regardless of room.
    let mut metrics: Vec<String> = configured.iter().map(metric_of).collect();
    metrics.sort();
    metrics.dedup();
    for metric in metrics {
        let plural = if metric.ends_with('s') { metric.clone() } else { format!("{metric}s") };
        let enum_phrases: Vec<String> = match locale {
            "fr" => vec![format!("toutes les {plural}"), format!("toutes les {metric}")],
            "en" => vec![format!("all {plural}"), format!("all the {plural}")],
            _ => Vec::new(),
        };
        rules.push(PatternRule {
            intent: format!("jeedom.read_all.{metric}"),
            phrases: enum_phrases,
            slots: Vec::new(),
        });
    }
    rules
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
                help: "Spoken name → Jeedom command id; room/kind/prefix filled by discovery"
                    .into(),
                default: String::new(),
                item_fields: vec![
                    ItemField {
                        key: "name".into(),
                        kind: FieldKind::String,
                        required: true,
                    },
                    ItemField {
                        key: "id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "unit".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "room".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "prefix".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "kind".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "on_label".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                    ItemField {
                        key: "off_label".into(),
                        kind: FieldKind::String,
                        required: false,
                    },
                ],
            },
        ],
    };
    Ok(serde_json::to_string(&schema)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, id: u64, unit: &str, room: &str, kind: &str, on: &str, off: &str) -> Sensor {
        Sensor {
            name: name.into(), id, unit: unit.into(), room: room.into(),
            kind: kind.into(), on_label: on.into(), off_label: off.into(),
            prefix: String::new(),
        }
    }

    fn sp(name: &str, id: u64, room: &str, prefix: &str) -> Sensor {
        Sensor {
            prefix: prefix.into(),
            ..s(name, id, "", room, "numeric", "", "")
        }
    }

    #[test]
    fn prefix_parses_cleans_and_defaults_empty() {
        let raw = r#"[{"name":"température d'alicia","id":7,"room":"alicia","prefix":"d’"},
                      {"name":"température du salon","id":8,"room":"salon"}]"#;
        let v = parse_sensors(raw);
        assert_eq!(
            v[0].prefix, "d'",
            "typographic apostrophe normalized to straight"
        );
        assert_eq!(v[1].prefix, "", "missing prefix defaults empty");
    }

    #[test]
    fn old_config_shape_still_parses() {
        let raw = r#"[{"name":"température du salon","id":142,"unit":"degrés"}]"#;
        let v: Vec<Sensor> = serde_json::from_str(raw).unwrap();
        assert_eq!(v[0].room, "");
        assert_eq!(v[0].kind, "");
        assert!(v[0].on_label.is_empty() && v[0].off_label.is_empty());
    }

    #[test]
    fn parsing_strips_symbols_from_spoken_fields() {
        // Jeedom discovery composes names/rooms that can carry icon
        // characters ("salon 🖴"); they must never reach generated patterns
        // or spoken answers. Units keep their symbols ("°C" must stay
        // speakable as degrés Celsius).
        let raw = r#"[{"name":"température 🌡 du salon","id":142,"unit":"°C","room":"salon 🖴"}]"#;
        let v = parse_sensors(raw);
        assert_eq!(v[0].name, "température du salon");
        assert_eq!(v[0].room, "salon");
        assert_eq!(v[0].unit, "°C");
        assert_eq!(
            enum_clause(&v[0], "21.5", false),
            "salon 21.5 °C",
            "spoken clause must be symbol-free apart from the unit"
        );
    }

    #[test]
    fn room_query_phrases_are_generated() {
        let list = vec![s("température du salon", 142, "degrés", "salon", "numeric", "", "")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules.iter().flat_map(|r| r.phrases.iter().map(String::as_str)).collect();
        assert!(all.contains(&"quelle température dans le salon"), "got: {all:?}");
        assert!(all.contains(&"température dans le salon"));
        // Natural full-sentence French: "quelle est la
        // température dans le salon" / "… du salon".
        assert!(all.contains(&"quelle est la température dans le salon"), "got: {all:?}");
        assert!(all.contains(&"quelle est la température du salon"), "got: {all:?}");
        assert!(all.contains(&"température du salon"));
        // the room rule routes to the literal per-sensor intent
        assert!(rules.iter().any(|r| r.intent == "jeedom.read.142"
            && r.phrases.iter().any(|p| p.contains("dans le salon"))));
    }

    #[test]
    fn enumeration_rule_per_metric() {
        let list = vec![
            s("température du salon", 142, "degrés", "salon", "numeric", "", ""),
            s("température de la chambre", 150, "degrés", "chambre", "numeric", "", ""),
            s("humidité du salon", 143, "%", "salon", "numeric", "", ""),
        ];
        let rules = rules_for("fr", &list);
        let enum_rules: Vec<_> = rules.iter().filter(|r| r.intent.starts_with("jeedom.read_all.")).collect();
        assert_eq!(enum_rules.len(), 2, "one per distinct metric: température, humidité");
        assert!(enum_rules.iter().any(|r| r.intent == "jeedom.read_all.température"
            && r.phrases.contains(&"toutes les températures".to_string())));
    }

    #[test]
    fn metric_word_strips_room_suffix() {
        assert_eq!(metric_of(&s("température du salon", 1, "", "salon", "", "", "")), "température");
        assert_eq!(metric_of(&s("température de la chambre", 1, "", "chambre", "", "", "")), "température");
        assert_eq!(metric_of(&s("capteur exotique", 1, "", "", "", "", "")), "capteur exotique");
    }

    #[test]
    fn enum_clause_labels_binary_sensors() {
        let temp = s("température du salon", 1, "degrés", "salon", "numeric", "", "");
        assert_eq!(enum_clause(&temp, "21.5", false), "salon 21.5 degrés");

        let door = s("porte du garage", 2, "", "garage", "binary", "ouverte", "fermée");
        assert_eq!(enum_clause(&door, "1", false), "garage ouverte");

        let presence = s("présence chambre", 3, "", "chambre", "binary", "", "");
        assert_eq!(enum_clause(&presence, "1", false), "chambre activé");
    }

    #[test]
    fn binary_reading_uses_labels_and_falls_back() {
        let door = s("porte du garage", 201, "", "garage", "binary", "ouverte", "fermée");
        assert_eq!(phrase_reading(&door, "1", false), "la porte du garage est ouverte");
        assert_eq!(phrase_reading(&door, "0", false), "la porte du garage est fermée");
        let plain = s("présence salon", 202, "", "salon", "binary", "", "");
        assert_eq!(phrase_reading(&plain, "1", false), "la présence salon est activé");
        let temp = s("température du salon", 142, "degrés", "salon", "numeric", "", "");
        assert_eq!(phrase_reading(&temp, "21.5", false), "la température du salon est de 21.5 degrés");
        assert_eq!(phrase_reading(&temp, "21.5", true), "the température du salon is 21.5 degrés");
    }
}
