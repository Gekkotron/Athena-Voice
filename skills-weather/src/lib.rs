//! `skills-weather` — Open-Meteo weather skill for Athena-Voice.
//!
//! Guest ABI (matches `crates/athena-voice-runtime/src/wasm/registry.rs`):
//! - `pattern_rules(locale) -> String`  (JSON of `Vec<PatternRule>`)
//! - `handle(intent_json) -> String`    (JSON of `Result<SkillResponse, SkillError>`)
//!
//! Per-skill config keys (all optional):
//! - `default_city` — fallback city when the utterance has no slot (defaults
//!   to `"Paris"`).
//! - `units` — reserved; the Speak template is currently Celsius-only.
//! - `geocoding_base_url` — base URL for the Open-Meteo geocoding API.
//! - `forecast_base_url` — base URL for the Open-Meteo forecast API.
//!
//! The `*_base_url` knobs exist so integration tests can point the skill at a
//! `wiremock` server on `127.0.0.1` without changing the skill source.

mod weather_code;

use athena_voice_skill_sdk::host::HostCtx;
use athena_voice_skill_sdk::{
    Intent, PatternRule, Skill, SkillError, SkillResponse, SlotKind, SlotSpec,
};
use extism_pdk::{FnResult, plugin_fn};
use serde::{Deserialize, Serialize};

const HARD_DEFAULT_CITY: &str = "Paris";
const GEOCODING_DEFAULT_BASE: &str = "https://geocoding-api.open-meteo.com";
const FORECAST_DEFAULT_BASE: &str = "https://api.open-meteo.com";

const HTTP_ERROR_SPEAK: &str = "désolé, le service météo est indisponible";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Geo {
    lat: f64,
    lon: f64,
    name: String,
}

struct WeatherSkill;

impl Skill for WeatherSkill {
    fn name(&self) -> &str {
        "weather"
    }

    fn pattern_rules(&self, locale: &str) -> Vec<PatternRule> {
        if locale != "fr" {
            return Vec::new();
        }
        let city_slot = vec![SlotSpec {
            name: "city".into(),
            kind: SlotKind::String,
        }];
        vec![
            PatternRule {
                intent: "weather.now".into(),
                phrases: vec![
                    "quel temps fait-il".into(),
                    "quel temps fait-il à {city}".into(),
                    "météo à {city}".into(),
                    "quelle est la température".into(),
                    "quelle est la température extérieure".into(),
                    "quelle température fait-il".into(),
                    "quelle température fait-il à {city}".into(),
                ],
                slots: city_slot.clone(),
            },
            PatternRule {
                intent: "weather.tomorrow".into(),
                phrases: vec![
                    "quel temps fera-t-il demain".into(),
                    "quel temps fera-t-il demain à {city}".into(),
                ],
                slots: city_slot,
            },
        ]
    }

    fn handle(&mut self, intent: Intent, ctx: &mut HostCtx) -> Result<SkillResponse, SkillError> {
        let requested_city = resolve_city(&intent, ctx);

        let geo = match resolve_geocoding(ctx, &requested_city) {
            Ok(Some(g)) => g,
            Ok(None) => {
                return Ok(SkillResponse::speak(format!(
                    "désolé, je ne trouve pas {requested_city}"
                )));
            }
            Err(msg) => return Ok(SkillResponse::speak(msg)),
        };

        match intent.name.as_str() {
            "weather.now" => weather_now(ctx, &geo),
            "weather.tomorrow" => weather_tomorrow(ctx, &geo),
            other => Err(SkillError::Custom(format!("unknown intent: {other}"))),
        }
    }
}

fn resolve_city(intent: &Intent, ctx: &HostCtx) -> String {
    let slot = intent
        .slots
        .get("city")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(s) = slot {
        return s.to_string();
    }
    ctx.config_get_toml("default_city")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| HARD_DEFAULT_CITY.to_string())
}

/// Resolves a city to `Geo` coordinates.
///
/// Order: state cache → geocoding call.
///
/// Cache TTL/refresh is a Plan 5 concern — for now cache entries live forever
/// and there is no negative-cache for unknown cities. `Ok(None)` means the
/// remote geocoding API returned zero results (unknown city); `Err(msg)`
/// carries a user-facing failure line for HTTP / decode errors.
fn resolve_geocoding(ctx: &HostCtx, city: &str) -> Result<Option<Geo>, String> {
    let key = format!("geo/{}", city.to_lowercase());
    if let Ok(Some(bytes)) = ctx.state_get(&key)
        && let Ok(cached) = serde_json::from_slice::<Geo>(&bytes)
    {
        return Ok(Some(cached));
    }

    let base = ctx
        .config_get_toml("geocoding_base_url")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| GEOCODING_DEFAULT_BASE.to_string());
    let base = base.trim_end_matches('/');
    let encoded = url_encode(city);
    let url = format!("{base}/v1/search?name={encoded}&language=fr&count=1");

    let value = match ctx.http_get_json(&url) {
        Ok(v) => v,
        Err(e) => {
            ctx.log("warn", &format!("weather: geocoding HTTP failed: {e}"));
            return Err(HTTP_ERROR_SPEAK.to_string());
        }
    };

    let results = value.get("results").and_then(|v| v.as_array());
    let Some(first) = results.and_then(|arr| arr.first()) else {
        return Ok(None);
    };

    let lat = first.get("latitude").and_then(serde_json::Value::as_f64);
    let lon = first.get("longitude").and_then(serde_json::Value::as_f64);
    let name = first
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(city)
        .to_string();

    let (Some(lat), Some(lon)) = (lat, lon) else {
        ctx.log(
            "error",
            "weather: geocoding response missing latitude/longitude",
        );
        return Err(HTTP_ERROR_SPEAK.to_string());
    };

    let geo = Geo { lat, lon, name };
    if let Ok(bytes) = serde_json::to_vec(&geo) {
        let _ = ctx.state_set(&key, &bytes);
    }
    Ok(Some(geo))
}

fn weather_now(ctx: &HostCtx, geo: &Geo) -> Result<SkillResponse, SkillError> {
    let value = match fetch_forecast(ctx, geo) {
        Ok(v) => v,
        Err(msg) => return Ok(SkillResponse::speak(msg)),
    };
    let current = match value.get("current") {
        Some(c) => c,
        None => {
            ctx.log("error", "weather: forecast response missing 'current'");
            return Ok(SkillResponse::speak(HTTP_ERROR_SPEAK));
        }
    };
    let temp = current
        .get("temperature_2m")
        .and_then(serde_json::Value::as_f64);
    let code = current
        .get("weather_code")
        .and_then(serde_json::Value::as_i64);
    let (Some(temp), Some(code)) = (temp, code) else {
        ctx.log(
            "error",
            "weather: 'current' missing temperature_2m or weather_code",
        );
        return Ok(SkillResponse::speak(HTTP_ERROR_SPEAK));
    };
    let temp_i = temp.round() as i64;
    let phrase = weather_code::fr_phrase(code);
    Ok(SkillResponse::speak(format!(
        "il fait {temp_i} degrés à {}, {phrase}",
        geo.name
    )))
}

fn weather_tomorrow(ctx: &HostCtx, geo: &Geo) -> Result<SkillResponse, SkillError> {
    let value = match fetch_forecast(ctx, geo) {
        Ok(v) => v,
        Err(msg) => return Ok(SkillResponse::speak(msg)),
    };
    let daily = match value.get("daily") {
        Some(d) => d,
        None => {
            ctx.log("error", "weather: forecast response missing 'daily'");
            return Ok(SkillResponse::speak(HTTP_ERROR_SPEAK));
        }
    };
    let min = daily
        .get("temperature_2m_min")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(serde_json::Value::as_f64);
    let max = daily
        .get("temperature_2m_max")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(serde_json::Value::as_f64);
    let code = daily
        .get("weather_code")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(serde_json::Value::as_i64);
    let (Some(min), Some(max), Some(code)) = (min, max, code) else {
        ctx.log(
            "error",
            "weather: 'daily' missing min/max/weather_code at index 0",
        );
        return Ok(SkillResponse::speak(HTTP_ERROR_SPEAK));
    };
    let min_i = min.round() as i64;
    let max_i = max.round() as i64;
    let phrase = weather_code::fr_phrase(code);
    Ok(SkillResponse::speak(format!(
        "demain à {}, il fera entre {min_i} et {max_i} degrés avec {phrase}",
        geo.name
    )))
}

fn fetch_forecast(ctx: &HostCtx, geo: &Geo) -> Result<serde_json::Value, String> {
    let base = ctx
        .config_get_toml("forecast_base_url")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| FORECAST_DEFAULT_BASE.to_string());
    let base = base.trim_end_matches('/');
    let url = format!(
        "{base}/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min,weather_code&timezone=auto",
        geo.lat, geo.lon
    );
    match ctx.http_get_json(&url) {
        Ok(v) => Ok(v),
        Err(e) => {
            ctx.log("warn", &format!("weather: forecast HTTP failed: {e}"));
            Err(HTTP_ERROR_SPEAK.to_string())
        }
    }
}

/// Minimal URL-encoding for the city name in the geocoding query string. We
/// only need to escape a handful of characters the Open-Meteo API is picky
/// about; anything else — including UTF-8 accented characters — is passed
/// through verbatim (Open-Meteo accepts UTF-8 in query strings).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '&' => out.push_str("%26"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '=' => out.push_str("%3D"),
            '+' => out.push_str("%2B"),
            _ => out.push(c),
        }
    }
    out
}

#[plugin_fn]
pub fn pattern_rules(locale: String) -> FnResult<String> {
    let rules = WeatherSkill.pattern_rules(&locale);
    Ok(serde_json::to_string(&rules)?)
}

#[plugin_fn]
pub fn handle(intent_json: String) -> FnResult<String> {
    let intent: Intent = serde_json::from_str(&intent_json)?;
    let mut skill = WeatherSkill;
    let mut c = HostCtx::for_testing();
    let result = skill.handle(intent, &mut c);
    Ok(serde_json::to_string(&result)?)
}
