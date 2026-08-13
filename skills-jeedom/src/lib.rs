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
//! - `actions` (optional) is a JSON-encoded array of on/off devices:
//!   `[{"name":"lumière du salon","on_id":124,"off_id":125,"confirm":false}, …]`
//!   — executing an action command uses the same GET as a read.
//! - `shutters` (optional) is a JSON-encoded array of roller shutters:
//!   `[{"name":"volet du salon","up_id":210,"down_id":211,"stop_id":212,
//!   "slider_id":213,"confirm":false}, …]` — `up_id`/`down_id` required;
//!   `stop_id`/`slider_id` optional (0 or absent = not configured).
//!   `confirm` gates open/close/position behind a spoken confirmation but
//!   never stop. Position runs the slider command with `&slider=N`
//!   (0 = closed, 100 = open, Jeedom's FLAP convention).
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

/// One controllable on/off device: two Jeedom action command ids behind a
/// single spoken name. Like sensors, `name` stores the FULL composed
/// spoken form ("lumière du salon"); room/prefix are auxiliary metadata.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ActionDevice {
    name: String,
    #[serde(default)]
    room: String,
    #[serde(default)]
    prefix: String,
    on_id: u64,
    off_id: u64,
    /// True → the assistant asks "Tu confirmes : … ?" and waits for a
    /// spoken oui/confirme before executing.
    #[serde(default)]
    confirm: bool,
}

static ACTIONS: OnceCell<Vec<ActionDevice>> = OnceCell::new();

fn parse_actions(raw: &str) -> Vec<ActionDevice> {
    let Ok(mut v) = serde_json::from_str::<Vec<ActionDevice>>(raw) else {
        return Vec::new();
    };
    for d in &mut v {
        d.name = clean_spoken(&d.name);
        d.room = clean_spoken(&d.room);
        d.prefix = clean_spoken(&d.prefix.replace('’', "'"));
    }
    v
}

/// One roller shutter: up/down Jeedom action command ids behind a single
/// spoken name, with optional stop and position-slider commands (0 = not
/// configured — Jeedom ids start at 1, and the admin UI leaves untouched
/// number cells absent). Like sensors, `name` stores the FULL composed
/// spoken form ("volet du salon").
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Shutter {
    name: String,
    #[serde(default)]
    room: String,
    #[serde(default)]
    prefix: String,
    up_id: u64,
    down_id: u64,
    #[serde(default)]
    stop_id: u64,
    #[serde(default)]
    slider_id: u64,
    /// True → open/close/position ask "Tu confirmes : … ?" first.
    /// Stop is never gated: stopping a moving shutter must be immediate.
    #[serde(default)]
    confirm: bool,
}

static SHUTTERS: OnceCell<Vec<Shutter>> = OnceCell::new();

fn parse_shutters(raw: &str) -> Vec<Shutter> {
    let Ok(mut v) = serde_json::from_str::<Vec<Shutter>>(raw) else {
        return Vec::new();
    };
    for s in &mut v {
        s.name = clean_spoken(&s.name);
        s.room = clean_spoken(&s.room);
        s.prefix = clean_spoken(&s.prefix.replace('’', "'"));
    }
    v
}

fn shutters(ctx: &HostCtx) -> &'static [Shutter] {
    SHUTTERS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("shutters").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            parse_shutters(&raw)
        })
        .as_slice()
}

fn actions(ctx: &HostCtx) -> &'static [ActionDevice] {
    ACTIONS
        .get_or_init(|| {
            let raw = ctx.config_get_toml("actions").unwrap_or_default();
            if raw.is_empty() {
                return Vec::new();
            }
            parse_actions(&raw)
        })
        .as_slice()
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
        let ctx = HostCtx::for_testing();
        let mut rules = rules_for(locale, sensors(&ctx));
        rules.extend(action_rules(locale, actions(&ctx)));
        rules.extend(shutter_rules(locale, shutters(&ctx)));
        if actions(&ctx).iter().any(|d| d.confirm) || shutters(&ctx).iter().any(|s| s.confirm) {
            rules.extend(confirm_rules(locale));
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

        // Enumeration intents ask for every sensor sharing a metric
        // ("toutes les températures") rather than a single sensor.
        if let Some(metric) = intent.name.strip_prefix("jeedom.read_all.") {
            let list = sensors(ctx);
            let matching: Vec<&Sensor> = list
                .iter()
                .filter(|s| metric_of(s) == metric || s.name.contains(metric))
                .collect();
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
                        let place = if sensor.room.is_empty() {
                            sensor.name.clone()
                        } else {
                            sensor.room.clone()
                        };
                        clauses.push(if en {
                            format!("{place} unavailable")
                        } else {
                            format!("{place} indisponible")
                        });
                    }
                }
            }
            return Ok(SkillResponse::speak(clauses.join(", ")));
        }

        // On/off device intents: the key riding in the intent name is the
        // device's on_id, for both directions.
        let turn = intent
            .name
            .strip_prefix("jeedom.turn_on.")
            .map(|k| (k, true))
            .or_else(|| {
                intent
                    .name
                    .strip_prefix("jeedom.turn_off.")
                    .map(|k| (k, false))
            });
        if let Some((key, on)) = turn {
            let Some(device) = key
                .parse::<u64>()
                .ok()
                .and_then(|k| actions(ctx).iter().find(|d| d.on_id == k))
            else {
                return Ok(SkillResponse::speak(if en {
                    "sorry, I don't know that device"
                } else {
                    "désolé, je ne connais pas cet appareil"
                }));
            };
            let cmd_id = if on { device.on_id } else { device.off_id };
            if device.confirm {
                let label = action_label(device, on, en);
                let pending = Pending {
                    cmd_id,
                    label: label.clone(),
                    slider: None,
                };
                if let Ok(bytes) = serde_json::to_vec(&pending) {
                    let _ = ctx.tmp_set(PENDING_KEY, &bytes, PENDING_TTL_SEC);
                }
                return Ok(SkillResponse::speak(if en {
                    format!("Confirm: {label}?")
                } else {
                    format!("Tu confirmes : {label} ?")
                }));
            }
            return Ok(done_or_error(exec_cmd(ctx, cmd_id), en));
        }

        // Group shutter intents run every configured shutter, unconditionally
        // (no confirmation: the command names its full scope already).
        if intent.name == "jeedom.shutter_open_all" || intent.name == "jeedom.shutter_close_all" {
            let open = intent.name.ends_with("open_all");
            let list = shutters(ctx);
            if list.is_empty() {
                return Ok(SkillResponse::speak(if en {
                    "sorry, I don't know that device"
                } else {
                    "désolé, je ne connais pas cet appareil"
                }));
            }
            let failed = list
                .iter()
                .filter(|s| exec_cmd(ctx, if open { s.up_id } else { s.down_id }).is_err())
                .count();
            return Ok(SkillResponse::speak(group_answer(list.len(), failed, en)));
        }

        // Per-shutter intents: the key riding in the intent name is up_id.
        let shutter_cmd = intent
            .name
            .strip_prefix("jeedom.shutter_open.")
            .map(|k| (k, ShutterCmd::Open))
            .or_else(|| {
                intent
                    .name
                    .strip_prefix("jeedom.shutter_close.")
                    .map(|k| (k, ShutterCmd::Close))
            })
            .or_else(|| {
                intent
                    .name
                    .strip_prefix("jeedom.shutter_stop.")
                    .map(|k| (k, ShutterCmd::Stop))
            })
            .or_else(|| {
                intent
                    .name
                    .strip_prefix("jeedom.shutter_pos.")
                    .map(|k| (k, ShutterCmd::Pos(0)))
            });
        if let Some((key, mut cmd)) = shutter_cmd {
            let Some(shutter) = key
                .parse::<u64>()
                .ok()
                .and_then(|k| shutters(ctx).iter().find(|s| s.up_id == k))
            else {
                return Ok(SkillResponse::speak(if en {
                    "sorry, I don't know that device"
                } else {
                    "désolé, je ne connais pas cet appareil"
                }));
            };
            if let ShutterCmd::Pos(_) = cmd {
                let Some(p) = intent.slots.get("position").and_then(slot_number) else {
                    return Ok(SkillResponse::speak(ask_position(en)));
                };
                cmd = ShutterCmd::Pos(p.clamp(0.0, 100.0) as u64);
            }
            let cmd_id = match cmd {
                ShutterCmd::Open => shutter.up_id,
                ShutterCmd::Close => shutter.down_id,
                ShutterCmd::Stop => shutter.stop_id,
                ShutterCmd::Pos(_) => shutter.slider_id,
            };
            if cmd_id == 0 {
                // Rules for stop/pos are only registered when the id is set,
                // so this is unreachable in practice — apologise defensively.
                return Ok(SkillResponse::speak(if en {
                    "sorry, I don't know that device"
                } else {
                    "désolé, je ne connais pas cet appareil"
                }));
            }
            // Stop is never gated behind confirmation.
            if shutter.confirm && !matches!(cmd, ShutterCmd::Stop) {
                let label = shutter_label(&shutter.name, &cmd, en);
                let slider = match cmd {
                    ShutterCmd::Pos(v) => Some(v),
                    _ => None,
                };
                let pending = Pending {
                    cmd_id,
                    label: label.clone(),
                    slider,
                };
                if let Ok(bytes) = serde_json::to_vec(&pending) {
                    let _ = ctx.tmp_set(PENDING_KEY, &bytes, PENDING_TTL_SEC);
                }
                return Ok(SkillResponse::speak(if en {
                    format!("Confirm: {label}?")
                } else {
                    format!("Tu confirmes : {label} ?")
                }));
            }
            let executed = match cmd {
                ShutterCmd::Pos(v) => exec_slider(ctx, cmd_id, v),
                _ => exec_cmd(ctx, cmd_id),
            };
            return Ok(done_or_error(executed, en));
        }

        if intent.name == "jeedom.confirm" {
            return Ok(match load_pending(ctx) {
                Some(p) => {
                    clear_pending(ctx);
                    let executed = match p.slider {
                        Some(v) => exec_slider(ctx, p.cmd_id, v),
                        None => exec_cmd(ctx, p.cmd_id),
                    };
                    done_or_error(executed, en)
                }
                None => SkillResponse::speak(if en {
                    "Nothing to confirm."
                } else {
                    "Rien à confirmer."
                }),
            });
        }
        if intent.name == "jeedom.cancel" {
            return Ok(match load_pending(ctx) {
                Some(_) => {
                    clear_pending(ctx);
                    SkillResponse::speak(if en { "Cancelled." } else { "Annulé." })
                }
                None => SkillResponse::speak(if en {
                    "Nothing to confirm."
                } else {
                    "Rien à confirmer."
                }),
            });
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
        if sensor.on_label.is_empty() {
            if en { "on" } else { "activé" }
        } else {
            &sensor.on_label
        }
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
    let unit = if sensor.unit.is_empty() {
        String::new()
    } else {
        format!(" {}", sensor.unit)
    };
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
    let place = if sensor.room.is_empty() {
        sensor.name.clone()
    } else {
        sensor.room.clone()
    };
    if sensor.kind == "binary" {
        format!("{place} {}", binary_label(sensor, value, en))
    } else {
        let unit = if sensor.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", sensor.unit)
        };
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
    // A configured prefix is stripped first — « température d'alicia » with
    // prefix « d' » must yield « température », and « d' » is not in the
    // hardcoded article fallback list.
    let prefix = sensor.prefix.trim().to_lowercase();
    let head = if !prefix.is_empty() && head.ends_with(&prefix) {
        head[..head.len() - prefix.len()].trim_end()
    } else {
        ["du", "de la", "de l’", "de l'", "de", "dans le", "dans la"]
            .iter()
            .find_map(|art| head.strip_suffix(art))
            .unwrap_or(head)
            .trim_end()
    };
    if head.is_empty() {
        name
    } else {
        head.to_string()
    }
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

/// Joins a French connector to a room: no space after an elided form
/// (« d'alicia »), one space otherwise (« du salon »).
fn join_prefix(prefix: &str, room: &str) -> String {
    if prefix.ends_with('\'') || prefix.ends_with('’') {
        format!("{prefix}{room}")
    } else {
        format!("{prefix} {room}")
    }
}

/// Definite article implied by a de-form prefix, for locative « dans … »
/// phrases. Unmapped prefixes (« d' », « chez ») get no dans-form —
/// « dans d'Alicia » must never exist.
fn dans_article(prefix: &str) -> Option<&'static str> {
    match prefix.trim() {
        "du" => Some("le"),
        "de la" => Some("la"),
        "de l'" | "de l’" => Some("l'"),
        _ => None,
    }
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
            "en" => vec![format!("what is the {name}"), format!("what's the {name}")],
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
                "fr" => {
                    if sensor.prefix.is_empty() {
                        literal_phrases.extend([
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
                        ]);
                    } else {
                        // The configured connector replaces the article
                        // guessing entirely; locative « dans » forms exist
                        // only when the prefix maps to a definite article.
                        let with_room = join_prefix(&sensor.prefix, room);
                        literal_phrases.extend([
                            format!("{metric} {with_room}"),
                            format!("quelle est la {metric} {with_room}"),
                        ]);
                        if let Some(article) = dans_article(&sensor.prefix) {
                            let loc = join_prefix(article, room);
                            literal_phrases.extend([
                                format!("{metric} dans {loc}"),
                                format!("quelle {metric} dans {loc}"),
                                format!("quelle est la {metric} dans {loc}"),
                            ]);
                        }
                    }
                }
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
        let plural = if metric.ends_with('s') {
            metric.clone()
        } else {
            format!("{metric}s")
        };
        let enum_phrases: Vec<String> = match locale {
            "fr" => vec![
                format!("toutes les {plural}"),
                format!("toutes les {metric}"),
            ],
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

/// Match rules for configured on/off devices, plus the global
/// confirm/cancel rules when any device requires confirmation. The device
/// key riding in the intent name is always `on_id`.
fn action_rules(locale: &str, devices: &[ActionDevice]) -> Vec<PatternRule> {
    if devices.is_empty() {
        return Vec::new();
    }
    let mut rules = Vec::new();
    for d in devices {
        let name = &d.name;
        let (on_phrases, off_phrases): (Vec<String>, Vec<String>) = match locale {
            "fr" => (
                vec![
                    format!("allume la {name}"),
                    format!("allume le {name}"),
                    format!("allume {name}"),
                    format!("active {name}"),
                ],
                vec![
                    format!("éteins la {name}"),
                    format!("éteins le {name}"),
                    format!("éteins {name}"),
                    format!("coupe {name}"),
                    format!("désactive {name}"),
                ],
            ),
            "en" => (
                vec![
                    format!("turn on the {name}"),
                    format!("turn on {name}"),
                    format!("switch on the {name}"),
                ],
                vec![
                    format!("turn off the {name}"),
                    format!("turn off {name}"),
                    format!("switch off the {name}"),
                ],
            ),
            _ => return Vec::new(),
        };
        rules.push(PatternRule {
            intent: format!("jeedom.turn_on.{}", d.on_id),
            phrases: on_phrases,
            slots: Vec::new(),
        });
        rules.push(PatternRule {
            intent: format!("jeedom.turn_off.{}", d.on_id),
            phrases: off_phrases,
            slots: Vec::new(),
        });
    }
    rules
}

/// The shared spoken confirm/cancel rules — registered once by
/// `pattern_rules` when any on/off device or shutter has `confirm: true`.
fn confirm_rules(locale: &str) -> Vec<PatternRule> {
    let (yes, no): (Vec<String>, Vec<String>) = match locale {
        "fr" => (
            vec!["oui".into(), "confirme".into(), "c'est confirmé".into()],
            vec!["non".into(), "annule".into()],
        ),
        "en" => (
            vec!["yes".into(), "confirm".into()],
            vec!["no".into(), "cancel".into()],
        ),
        _ => return Vec::new(),
    };
    vec![
        PatternRule {
            intent: "jeedom.confirm".into(),
            phrases: yes,
            slots: Vec::new(),
        },
        PatternRule {
            intent: "jeedom.cancel".into(),
            phrases: no,
            slots: Vec::new(),
        },
    ]
}

/// Match rules for configured shutters. Stop and position rules exist only
/// for shutters that configured those command ids; the two group rules are
/// registered once whenever any shutter exists. Key = `up_id`.
fn shutter_rules(locale: &str, list: &[Shutter]) -> Vec<PatternRule> {
    if list.is_empty() {
        return Vec::new();
    }
    let mut rules = Vec::new();
    for s in list {
        let name = &s.name;
        let (open, close, stop, pos): (Vec<String>, Vec<String>, Vec<String>, Vec<String>) =
            match locale {
                "fr" => (
                    vec![
                        format!("ouvre le {name}"),
                        format!("ouvre la {name}"),
                        format!("ouvre {name}"),
                        format!("monte le {name}"),
                        format!("lève le {name}"),
                    ],
                    vec![
                        format!("ferme le {name}"),
                        format!("ferme la {name}"),
                        format!("ferme {name}"),
                        format!("descends le {name}"),
                        format!("baisse le {name}"),
                    ],
                    vec![
                        format!("stop le {name}"),
                        format!("stop {name}"),
                        format!("arrête le {name}"),
                    ],
                    vec![
                        format!("ouvre le {name} à {{position}}"),
                        format!("ouvre le {name} à {{position}} pour cent"),
                        format!("mets le {name} à {{position}}"),
                        format!("mets le {name} à {{position}} pour cent"),
                    ],
                ),
                "en" => (
                    vec![
                        format!("open the {name}"),
                        format!("open {name}"),
                        format!("raise the {name}"),
                    ],
                    vec![
                        format!("close the {name}"),
                        format!("close {name}"),
                        format!("lower the {name}"),
                    ],
                    vec![format!("stop the {name}"), format!("stop {name}")],
                    vec![
                        format!("set the {name} to {{position}}"),
                        format!("set the {name} to {{position}} percent"),
                        format!("open the {name} to {{position}} percent"),
                    ],
                ),
                _ => return Vec::new(),
            };
        rules.push(PatternRule {
            intent: format!("jeedom.shutter_open.{}", s.up_id),
            phrases: open,
            slots: Vec::new(),
        });
        rules.push(PatternRule {
            intent: format!("jeedom.shutter_close.{}", s.up_id),
            phrases: close,
            slots: Vec::new(),
        });
        if s.stop_id != 0 {
            rules.push(PatternRule {
                intent: format!("jeedom.shutter_stop.{}", s.up_id),
                phrases: stop,
                slots: Vec::new(),
            });
        }
        if s.slider_id != 0 {
            rules.push(PatternRule {
                intent: format!("jeedom.shutter_pos.{}", s.up_id),
                phrases: pos,
                slots: vec![SlotSpec {
                    name: "position".into(),
                    kind: SlotKind::Number,
                }],
            });
        }
    }
    let (open_all, close_all): (Vec<String>, Vec<String>) = match locale {
        "fr" => (
            vec!["ouvre tous les volets".into(), "ouvre les volets".into()],
            vec!["ferme tous les volets".into(), "ferme les volets".into()],
        ),
        "en" => (
            vec!["open all the shutters".into(), "open the shutters".into()],
            vec!["close all the shutters".into(), "close the shutters".into()],
        ),
        _ => (Vec::new(), Vec::new()),
    };
    rules.push(PatternRule {
        intent: "jeedom.shutter_open_all".into(),
        phrases: open_all,
        slots: Vec::new(),
    });
    rules.push(PatternRule {
        intent: "jeedom.shutter_close_all".into(),
        phrases: close_all,
        slots: Vec::new(),
    });
    rules
}

/// Builds the authenticated simple-API URL for one command id.
fn jeedom_url(ctx: &HostCtx, id: u64) -> Result<String, ()> {
    let base = ctx
        .config_get_toml("base_url")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    let api_key = ctx
        .config_get_toml("api_key")
        .filter(|s| !s.is_empty())
        .ok_or(())?;
    Ok(format!(
        "{}/core/api/jeeApi.php?apikey={api_key}&type=cmd&id={id}",
        base.trim_end_matches('/'),
    ))
}

/// Executes one Jeedom ACTION command — same GET as a read; for an
/// action-type command the call runs it. The body (plain "ok", empty, or
/// JSON) is irrelevant: any 2xx counts as executed.
fn exec_cmd(ctx: &HostCtx, id: u64) -> Result<(), ()> {
    let url = jeedom_url(ctx, id)?;
    match ctx.http_get_json(&url) {
        Ok(_) => Ok(()),
        Err(e) => {
            ctx.log("warn", &format!("jeedom: action exec failed: {e}"));
            Err(())
        }
    }
}

/// Pending confirmation, stored in the skill's tmp KV. The tmp store has
/// no delete: "clear" = overwrite with an EMPTY payload (1 s TTL), and
/// every reader treats an empty payload as absent.
#[derive(Debug, serde::Serialize, Deserialize)]
struct Pending {
    cmd_id: u64,
    label: String,
    /// Set for position commands: confirm executes cmd_id as a slider.
    #[serde(default)]
    slider: Option<u64>,
}

/// What a shutter intent asks for; `Pos` carries the clamped 0–100 target.
/// `Copy` (payload is a bare u64) — the handler matches it by value twice.
#[derive(Debug, Clone, Copy)]
enum ShutterCmd {
    Open,
    Close,
    Stop,
    Pos(u64),
}

fn shutter_label(name: &str, cmd: &ShutterCmd, en: bool) -> String {
    match (cmd, en) {
        (ShutterCmd::Open, false) => format!("ouvrir {name}"),
        (ShutterCmd::Close, false) => format!("fermer {name}"),
        (ShutterCmd::Stop, false) => format!("arrêter {name}"),
        (ShutterCmd::Pos(v), false) => format!("mettre {name} à {v} pour cent"),
        (ShutterCmd::Open, true) => format!("open {name}"),
        (ShutterCmd::Close, true) => format!("close {name}"),
        (ShutterCmd::Stop, true) => format!("stop {name}"),
        (ShutterCmd::Pos(v), true) => format!("set {name} to {v} percent"),
    }
}

fn ask_position(en: bool) -> &'static str {
    if en {
        "To what position, in percent?"
    } else {
        "À quelle position, en pourcentage ?"
    }
}

/// The matcher inserts slot values as JSON strings; be tolerant of numbers.
fn slot_number(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn group_answer(total: usize, failed: usize, en: bool) -> String {
    if failed == 0 {
        return if en { "Done." } else { "C'est fait." }.into();
    }
    if failed >= total {
        return if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        }
        .into();
    }
    match (en, failed) {
        (false, 1) => "C'est fait, mais un volet n'a pas répondu.".into(),
        (false, n) => format!("C'est fait, mais {n} volets n'ont pas répondu."),
        (true, 1) => "Done, but 1 shutter did not respond.".into(),
        (true, n) => format!("Done, but {n} shutters did not respond."),
    }
}

/// Executes a Jeedom slider action command: same authenticated GET with the
/// target value as `&slider=`.
fn exec_slider(ctx: &HostCtx, id: u64, value: u64) -> Result<(), ()> {
    let url = jeedom_url(ctx, id)?;
    match ctx.http_get_json(&format!("{url}&slider={value}")) {
        Ok(_) => Ok(()),
        Err(e) => {
            ctx.log("warn", &format!("jeedom: slider exec failed: {e}"));
            Err(())
        }
    }
}

const PENDING_KEY: &str = "pending_action";
const PENDING_TTL_SEC: u64 = 30;

fn action_label(d: &ActionDevice, on: bool, en: bool) -> String {
    match (on, en) {
        (true, false) => format!("allumer {}", d.name),
        (false, false) => format!("éteindre {}", d.name),
        (true, true) => format!("turn on {}", d.name),
        (false, true) => format!("turn off {}", d.name),
    }
}

fn load_pending(ctx: &HostCtx) -> Option<Pending> {
    let bytes = ctx.tmp_get(PENDING_KEY).ok().flatten()?;
    if bytes.is_empty() {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn clear_pending(ctx: &HostCtx) {
    let _ = ctx.tmp_set(PENDING_KEY, b"", 1);
}

fn done_or_error(executed: Result<(), ()>, en: bool) -> SkillResponse {
    match executed {
        Ok(()) => SkillResponse::speak(if en { "Done." } else { "C'est fait." }),
        Err(()) => SkillResponse::speak(if en {
            "sorry, I can't reach Jeedom right now"
        } else {
            "désolé, je n'arrive pas à joindre Jeedom"
        }),
    }
}

/// Reads one command value through Jeedom's simple HTTP GET API.
fn read_value(ctx: &HostCtx, sensor: &Sensor) -> Result<String, ()> {
    let url = jeedom_url(ctx, sensor.id)?;
    match ctx.http_get_json(&url) {
        // Numeric sensors come back as a bare JSON scalar; be tolerant of
        // string-wrapped numbers and `{"value": …}` envelopes too.
        Ok(v) => {
            let value = v.get("value").cloned().unwrap_or(v);
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
            ConfigField {
                key: "actions".into(),
                label: "Actions".into(),
                kind: FieldKind::List,
                required: false,
                help: "On/off devices: spoken name → Jeedom on/off action command ids".into(),
                default: String::new(),
                item_fields: vec![
                    ItemField {
                        key: "name".into(),
                        kind: FieldKind::String,
                        required: true,
                    },
                    ItemField {
                        key: "on_id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "off_id".into(),
                        kind: FieldKind::Number,
                        required: true,
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
                        key: "confirm".into(),
                        kind: FieldKind::Bool,
                        required: false,
                    },
                ],
            },
            ConfigField {
                key: "shutters".into(),
                label: "Shutters".into(),
                kind: FieldKind::List,
                required: false,
                help: "Roller shutters: spoken name → Jeedom up/down action ids; stop/slider optional".into(),
                default: String::new(),
                item_fields: vec![
                    ItemField {
                        key: "name".into(),
                        kind: FieldKind::String,
                        required: true,
                    },
                    ItemField {
                        key: "up_id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "down_id".into(),
                        kind: FieldKind::Number,
                        required: true,
                    },
                    ItemField {
                        key: "stop_id".into(),
                        kind: FieldKind::Number,
                        required: false,
                    },
                    ItemField {
                        key: "slider_id".into(),
                        kind: FieldKind::Number,
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
                        key: "confirm".into(),
                        kind: FieldKind::Bool,
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
            name: name.into(),
            id,
            unit: unit.into(),
            room: room.into(),
            kind: kind.into(),
            on_label: on.into(),
            off_label: off.into(),
            prefix: String::new(),
        }
    }

    fn sp(name: &str, id: u64, room: &str, prefix: &str) -> Sensor {
        Sensor {
            prefix: prefix.into(),
            ..s(name, id, "", room, "numeric", "", "")
        }
    }

    fn a(name: &str, on_id: u64, off_id: u64, confirm: bool) -> ActionDevice {
        ActionDevice {
            name: name.into(),
            room: String::new(),
            prefix: String::new(),
            on_id,
            off_id,
            confirm,
        }
    }

    fn sh(name: &str, up_id: u64, down_id: u64) -> Shutter {
        Shutter {
            name: name.into(),
            room: String::new(),
            prefix: String::new(),
            up_id,
            down_id,
            stop_id: 0,
            slider_id: 0,
            confirm: false,
        }
    }

    #[test]
    fn shutters_parse_cleans_and_defaults() {
        let raw = r#"[{"name":"volet 🪟 du salon","up_id":210,"down_id":211},
                      {"name":"volet de la chambre","room":"chambre","prefix":"de la",
                       "up_id":220,"down_id":221,"stop_id":222,"slider_id":223,"confirm":true}]"#;
        let v = parse_shutters(raw);
        assert_eq!(v[0].name, "volet du salon", "symbols stripped");
        assert_eq!(v[0].stop_id, 0, "stop_id defaults to 0 = unset");
        assert_eq!(v[0].slider_id, 0, "slider_id defaults to 0 = unset");
        assert!(!v[0].confirm);
        assert_eq!(v[1].stop_id, 222);
        assert_eq!(v[1].slider_id, 223);
        assert!(v[1].confirm);
        assert_eq!(parse_shutters("not json"), Vec::<Shutter>::new().as_slice());
    }

    #[test]
    fn actions_parse_cleans_and_defaults() {
        let raw = r#"[{"name":"lumière 💡 du salon","on_id":124,"off_id":125},
                      {"name":"prise garage","room":"garage","on_id":7,"off_id":8,"confirm":true}]"#;
        let v = parse_actions(raw);
        assert_eq!(v[0].name, "lumière du salon", "symbols stripped");
        assert!(!v[0].confirm, "confirm defaults to false");
        assert!(v[1].confirm);
        assert_eq!(
            parse_actions("not json"),
            Vec::<ActionDevice>::new().as_slice()
        );
    }

    #[test]
    fn action_rules_generate_on_off_phrases() {
        let list = vec![a("lumière du salon", 124, 125, false)];
        let rules = action_rules("fr", &list);
        let on = rules
            .iter()
            .find(|r| r.intent == "jeedom.turn_on.124")
            .unwrap();
        assert!(
            on.phrases
                .contains(&"allume la lumière du salon".to_string()),
            "got: {:?}",
            on.phrases
        );
        assert!(on.phrases.contains(&"allume lumière du salon".to_string()));
        let off = rules
            .iter()
            .find(|r| r.intent == "jeedom.turn_off.124")
            .unwrap();
        assert!(
            off.phrases
                .contains(&"éteins la lumière du salon".to_string()),
            "got: {:?}",
            off.phrases
        );

        let en = action_rules("en", &list);
        assert!(en.iter().any(|r| {
            r.intent == "jeedom.turn_on.124"
                && r.phrases
                    .contains(&"turn on the lumière du salon".to_string())
        }));
    }

    #[test]
    fn shutter_rules_generate_open_close_phrases() {
        let list = vec![sh("volet du salon", 210, 211)];
        let rules = shutter_rules("fr", &list);
        let open = rules
            .iter()
            .find(|r| r.intent == "jeedom.shutter_open.210")
            .unwrap();
        assert!(
            open.phrases.contains(&"ouvre le volet du salon".to_string()),
            "got: {:?}",
            open.phrases
        );
        assert!(open.phrases.contains(&"monte le volet du salon".to_string()));
        let close = rules
            .iter()
            .find(|r| r.intent == "jeedom.shutter_close.210")
            .unwrap();
        assert!(
            close.phrases.contains(&"ferme le volet du salon".to_string()),
            "got: {:?}",
            close.phrases
        );
        assert!(close.phrases.contains(&"baisse le volet du salon".to_string()));

        let en = shutter_rules("en", &list);
        assert!(en.iter().any(|r| r.intent == "jeedom.shutter_open.210"
            && r.phrases.contains(&"open the volet du salon".to_string())));
    }

    #[test]
    fn shutter_stop_and_pos_rules_require_their_ids() {
        let plain = vec![sh("volet du salon", 210, 211)];
        let rules = shutter_rules("fr", &plain);
        assert!(!rules.iter().any(|r| r.intent.starts_with("jeedom.shutter_stop.")));
        assert!(!rules.iter().any(|r| r.intent.starts_with("jeedom.shutter_pos.")));

        let mut full = sh("volet du salon", 210, 211);
        full.stop_id = 212;
        full.slider_id = 213;
        let rules = shutter_rules("fr", &[full]);
        let stop = rules
            .iter()
            .find(|r| r.intent == "jeedom.shutter_stop.210")
            .unwrap();
        assert!(
            stop.phrases.contains(&"stop le volet du salon".to_string()),
            "got: {:?}",
            stop.phrases
        );
        let pos = rules
            .iter()
            .find(|r| r.intent == "jeedom.shutter_pos.210")
            .unwrap();
        assert!(
            pos.phrases
                .contains(&"ouvre le volet du salon à {position}".to_string()),
            "got: {:?}",
            pos.phrases
        );
        assert_eq!(pos.slots.len(), 1);
        assert_eq!(pos.slots[0].name, "position");
        assert!(matches!(pos.slots[0].kind, SlotKind::Number));
    }

    #[test]
    fn shutter_group_rules_exist_once() {
        let list = vec![sh("volet du salon", 210, 211), sh("volet de la chambre", 220, 221)];
        let rules = shutter_rules("fr", &list);
        let all_open: Vec<_> = rules
            .iter()
            .filter(|r| r.intent == "jeedom.shutter_open_all")
            .collect();
        assert_eq!(all_open.len(), 1, "one group rule regardless of shutter count");
        assert!(all_open[0].phrases.contains(&"ouvre tous les volets".to_string()));
        assert!(rules.iter().any(|r| r.intent == "jeedom.shutter_close_all"
            && r.phrases.contains(&"ferme tous les volets".to_string())));
        assert!(shutter_rules("fr", &[]).is_empty(), "no shutters, no rules");
    }

    #[test]
    fn action_rules_carry_no_confirm_rules() {
        // Confirm/cancel are shared between on/off devices and shutters and are
        // registered once by pattern_rules — never by action_rules itself.
        let confirmed = vec![a("portail", 30, 31, true)];
        assert!(
            !action_rules("fr", &confirmed)
                .iter()
                .any(|r| r.intent == "jeedom.confirm" || r.intent == "jeedom.cancel")
        );
    }

    #[test]
    fn confirm_rules_cover_both_locales() {
        let fr = confirm_rules("fr");
        let confirm = fr.iter().find(|r| r.intent == "jeedom.confirm").unwrap();
        assert!(confirm.phrases.contains(&"oui".to_string()));
        assert!(
            fr.iter()
                .any(|r| r.intent == "jeedom.cancel" && r.phrases.contains(&"annule".to_string()))
        );
        let en = confirm_rules("en");
        assert!(
            en.iter()
                .any(|r| r.intent == "jeedom.confirm" && r.phrases.contains(&"yes".to_string()))
        );
        assert!(confirm_rules("de").is_empty(), "unknown locale yields nothing");
    }

    #[test]
    fn no_action_rules_without_devices() {
        assert!(action_rules("fr", &[]).is_empty());
    }

    #[test]
    fn action_labels_phrase_both_locales() {
        let d = a("lumière du salon", 124, 125, true);
        assert_eq!(action_label(&d, true, false), "allumer lumière du salon");
        assert_eq!(action_label(&d, false, false), "éteindre lumière du salon");
        assert_eq!(action_label(&d, true, true), "turn on lumière du salon");
        assert_eq!(action_label(&d, false, true), "turn off lumière du salon");
    }

    #[test]
    fn pending_roundtrips_through_json() {
        let p = Pending {
            cmd_id: 124,
            label: "allumer lumière du salon".into(),
            slider: None,
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: Pending = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.cmd_id, 124);
        assert_eq!(back.label, p.label);
    }

    #[test]
    fn shutter_labels_phrase_both_locales() {
        assert_eq!(
            shutter_label("volet du salon", &ShutterCmd::Open, false),
            "ouvrir volet du salon"
        );
        assert_eq!(
            shutter_label("volet du salon", &ShutterCmd::Close, false),
            "fermer volet du salon"
        );
        assert_eq!(
            shutter_label("volet du salon", &ShutterCmd::Pos(50), false),
            "mettre volet du salon à 50 pour cent"
        );
        assert_eq!(
            shutter_label("volet du salon", &ShutterCmd::Open, true),
            "open volet du salon"
        );
        assert_eq!(
            shutter_label("volet du salon", &ShutterCmd::Pos(50), true),
            "set volet du salon to 50 percent"
        );
    }

    #[test]
    fn pending_slider_roundtrips_and_defaults() {
        // Old payloads (no slider field) must still load — the on/off flow
        // stores them and both flows share the same tmp key.
        let old = br#"{"cmd_id":124,"label":"allumer lampe"}"#;
        let p: Pending = serde_json::from_slice(old).unwrap();
        assert_eq!(p.slider, None);
        let new = Pending {
            cmd_id: 213,
            label: "mettre volet à 50 pour cent".into(),
            slider: Some(50),
        };
        let bytes = serde_json::to_vec(&new).unwrap();
        let back: Pending = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.slider, Some(50));
    }

    #[test]
    fn slot_number_reads_strings_and_numbers() {
        assert_eq!(slot_number(&serde_json::json!("50")), Some(50.0));
        assert_eq!(slot_number(&serde_json::json!(" 50 ")), Some(50.0));
        assert_eq!(slot_number(&serde_json::json!(50)), Some(50.0));
        assert_eq!(slot_number(&serde_json::json!("cinquante")), None);
        assert_eq!(slot_number(&serde_json::Value::Null), None);
    }

    #[test]
    fn group_answer_phrasing() {
        assert_eq!(group_answer(3, 0, false), "C'est fait.");
        assert_eq!(group_answer(3, 3, false), "désolé, je n'arrive pas à joindre Jeedom");
        assert_eq!(group_answer(3, 1, false), "C'est fait, mais un volet n'a pas répondu.");
        assert_eq!(group_answer(3, 2, false), "C'est fait, mais 2 volets n'ont pas répondu.");
        assert_eq!(group_answer(3, 1, true), "Done, but 1 shutter did not respond.");
        assert_eq!(group_answer(3, 2, true), "Done, but 2 shutters did not respond.");
    }

    #[test]
    fn shutter_intent_for_unknown_device_apologises() {
        // Host-side config is empty, so no shutter matches key 999.
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.shutter_open.999".into(),
            slots: Default::default(),
            locale: "fr".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "désolé, je ne connais pas cet appareil");
    }

    #[test]
    fn shutter_pos_without_number_reasks() {
        // The re-ask path needs configured shutters, which for_testing cannot
        // supply — the copy is pinned via ask_position() directly instead.
        assert_eq!(ask_position(false), "À quelle position, en pourcentage ?");
        assert_eq!(ask_position(true), "To what position, in percent?");
    }

    fn speak_text(r: SkillResponse) -> String {
        match r {
            SkillResponse::Speak { text } => text,
            other => panic!("expected Speak, got {other:?}"),
        }
    }

    #[test]
    fn confirm_with_nothing_pending_says_so() {
        // Host-side tmp_get is always None — exactly the nothing-pending path.
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.confirm".into(),
            slots: Default::default(),
            locale: "fr".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "Rien à confirmer.");
    }

    #[test]
    fn cancel_with_nothing_pending_says_so() {
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.cancel".into(),
            slots: Default::default(),
            locale: "en".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "Nothing to confirm.");
    }

    #[test]
    fn turn_intent_for_unknown_device_apologises() {
        // Host-side config is empty, so no device matches key 999.
        let mut ctx = HostCtx::for_testing();
        let intent = Intent {
            name: "jeedom.turn_on.999".into(),
            slots: Default::default(),
            locale: "fr".into(),
        };
        let r = JeedomSkill.handle(intent, &mut ctx).unwrap();
        assert_eq!(speak_text(r), "désolé, je ne connais pas cet appareil");
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
        let list = vec![s(
            "température du salon",
            142,
            "degrés",
            "salon",
            "numeric",
            "",
            "",
        )];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules
            .iter()
            .flat_map(|r| r.phrases.iter().map(String::as_str))
            .collect();
        assert!(
            all.contains(&"quelle température dans le salon"),
            "got: {all:?}"
        );
        assert!(all.contains(&"température dans le salon"));
        // Natural full-sentence French: "quelle est la
        // température dans le salon" / "… du salon".
        assert!(
            all.contains(&"quelle est la température dans le salon"),
            "got: {all:?}"
        );
        assert!(
            all.contains(&"quelle est la température du salon"),
            "got: {all:?}"
        );
        assert!(all.contains(&"température du salon"));
        // the room rule routes to the literal per-sensor intent
        assert!(rules.iter().any(|r| r.intent == "jeedom.read.142"
            && r.phrases.iter().any(|p| p.contains("dans le salon"))));
    }

    #[test]
    fn prefix_generates_elided_phrases_without_dans_forms() {
        let list = vec![sp("température d'alicia", 7, "alicia", "d'")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules
            .iter()
            .flat_map(|r| r.phrases.iter().map(String::as_str))
            .collect();
        assert!(
            all.contains(&"quelle est la température d'alicia"),
            "got: {all:?}"
        );
        assert!(all.contains(&"température d'alicia"), "got: {all:?}");
        assert!(
            !all.iter().any(|p| p.contains("dans")),
            "no dans-form for an unmapped prefix: {all:?}"
        );
        assert!(
            !all.iter()
                .any(|p| p.contains("du alicia") || p.contains("de la alicia")),
            "legacy article enumeration must be gone when a prefix is set: {all:?}"
        );
    }

    #[test]
    fn prefix_du_keeps_locative_dans_forms() {
        let list = vec![sp("température du salon", 142, "salon", "du")];
        let rules = rules_for("fr", &list);
        let all: Vec<&str> = rules
            .iter()
            .flat_map(|r| r.phrases.iter().map(String::as_str))
            .collect();
        assert!(all.contains(&"température du salon"), "got: {all:?}");
        assert!(
            all.contains(&"quelle est la température du salon"),
            "got: {all:?}"
        );
        assert!(all.contains(&"température dans le salon"), "got: {all:?}");
        assert!(
            all.contains(&"quelle température dans le salon"),
            "got: {all:?}"
        );
        assert!(
            all.contains(&"quelle est la température dans le salon"),
            "got: {all:?}"
        );
        assert!(
            !all.contains(&"température de la salon"),
            "wrong-gender enumeration gone when prefix set: {all:?}"
        );
    }

    #[test]
    fn metric_word_strips_configured_prefix() {
        assert_eq!(
            metric_of(&sp("température d'alicia", 7, "alicia", "d'")),
            "température"
        );
    }

    #[test]
    fn enumeration_rule_per_metric() {
        let list = vec![
            s(
                "température du salon",
                142,
                "degrés",
                "salon",
                "numeric",
                "",
                "",
            ),
            s(
                "température de la chambre",
                150,
                "degrés",
                "chambre",
                "numeric",
                "",
                "",
            ),
            s("humidité du salon", 143, "%", "salon", "numeric", "", ""),
        ];
        let rules = rules_for("fr", &list);
        let enum_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.intent.starts_with("jeedom.read_all."))
            .collect();
        assert_eq!(
            enum_rules.len(),
            2,
            "one per distinct metric: température, humidité"
        );
        assert!(
            enum_rules
                .iter()
                .any(|r| r.intent == "jeedom.read_all.température"
                    && r.phrases.contains(&"toutes les températures".to_string()))
        );
    }

    #[test]
    fn metric_word_strips_room_suffix() {
        assert_eq!(
            metric_of(&s("température du salon", 1, "", "salon", "", "", "")),
            "température"
        );
        assert_eq!(
            metric_of(&s(
                "température de la chambre",
                1,
                "",
                "chambre",
                "",
                "",
                ""
            )),
            "température"
        );
        assert_eq!(
            metric_of(&s("capteur exotique", 1, "", "", "", "", "")),
            "capteur exotique"
        );
    }

    #[test]
    fn enum_clause_labels_binary_sensors() {
        let temp = s(
            "température du salon",
            1,
            "degrés",
            "salon",
            "numeric",
            "",
            "",
        );
        assert_eq!(enum_clause(&temp, "21.5", false), "salon 21.5 degrés");

        let door = s(
            "porte du garage",
            2,
            "",
            "garage",
            "binary",
            "ouverte",
            "fermée",
        );
        assert_eq!(enum_clause(&door, "1", false), "garage ouverte");

        let presence = s("présence chambre", 3, "", "chambre", "binary", "", "");
        assert_eq!(enum_clause(&presence, "1", false), "chambre activé");
    }

    #[test]
    fn binary_reading_uses_labels_and_falls_back() {
        let door = s(
            "porte du garage",
            201,
            "",
            "garage",
            "binary",
            "ouverte",
            "fermée",
        );
        assert_eq!(
            phrase_reading(&door, "1", false),
            "la porte du garage est ouverte"
        );
        assert_eq!(
            phrase_reading(&door, "0", false),
            "la porte du garage est fermée"
        );
        let plain = s("présence salon", 202, "", "salon", "binary", "", "");
        assert_eq!(
            phrase_reading(&plain, "1", false),
            "la présence salon est activé"
        );
        let temp = s(
            "température du salon",
            142,
            "degrés",
            "salon",
            "numeric",
            "",
            "",
        );
        assert_eq!(
            phrase_reading(&temp, "21.5", false),
            "la température du salon est de 21.5 degrés"
        );
        assert_eq!(
            phrase_reading(&temp, "21.5", true),
            "the température du salon is 21.5 degrés"
        );
    }
}
