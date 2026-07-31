use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RetentionConfig {
    #[serde(default)]
    pub gc_after_sec: Option<u64>,
}

use figment::providers::{Env, Format, Toml};
use figment::{Error as FigmentError, Figment};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use athena_voice_core::ids::Locale;
use athena_voice_providers::ProviderConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub locales: Vec<Locale>,
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub mqtt: MqttConfig,
    pub providers: ProviderConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub assist: Option<AssistConfig>,
}

/// `[skills]` section: WASM skill loader configuration.
///
/// `dir` names the directory scanned for `*.wasm` skills at startup — empty
/// (or a missing directory) yields Plan 3 behaviour with zero skills loaded.
/// Per-skill entries live in the `per_skill` map, populated via TOML
/// sub-tables `[skills.<name>]` (flattened here so `dir` and skill names sit
/// side-by-side in the same section).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SkillsConfig {
    #[serde(default)]
    pub dir: Option<PathBuf>,
    /// When true, the runtime spawns a filesystem watcher on `dir` and
    /// live-reloads individual `*.wasm` files as they change. Defaults to
    /// false so production keeps a stable, snapshot-at-startup skill set.
    #[serde(default)]
    pub hot_reload: bool,
    /// Directory of prebuilt `.wasm` artifacts offered by the web UI's
    /// "install bundled skill" picker. Unset hides the picker.
    #[serde(default)]
    pub bundled_dir: Option<PathBuf>,
    #[serde(flatten, default)]
    pub per_skill: HashMap<String, PerSkillConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PerSkillConfig {
    #[serde(default)]
    pub http_allowlist: Vec<String>,
    /// Extra MQTT topic filters (MQTT topic-filter grammar with `+` / `#`)
    /// the skill may publish to on top of its default
    /// `athena/skills/<name>/*` namespace. Empty leaves the default ACL
    /// untouched.
    #[serde(default)]
    pub mqtt_publish_allowlist: Vec<String>,
    #[serde(default)]
    pub config: HashMap<String, String>,
    /// Optional TTL in seconds for keys set by this skill. Keys whose
    /// stored timestamp is older than `now_sec - gc_after_sec` are deleted
    /// automatically on the next write. Missing or 0 disables GC.
    #[serde(default)]
    pub retention: RetentionConfig,
    /// Optional INI config file served verbatim to the skill via
    /// `host_config_get` (takes precedence over the `config` map).
    #[serde(default)]
    pub config_file: Option<String>,
}

/// `[assist]` section: text bridge for the owner's home-automation app.
/// Absent block = bridge off; `enabled = false` also turns it off without
/// deleting the section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssistConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_assist_prefix")]
    pub topic_prefix: String,
    #[serde(default = "default_assist_locale")]
    pub locale: Locale,
}

fn default_true() -> bool {
    true
}

fn default_assist_prefix() -> String {
    "assist".into()
}

fn default_assist_locale() -> Locale {
    Locale::new("fr").expect("static locale")
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Sessions with no inbound audio/text for this many seconds are closed
    /// by the runtime's reaper (satellite died without sending `end`).
    #[serde(default = "default_session_idle_secs")]
    pub session_idle_secs: u64,
}

fn default_session_idle_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub database_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,
}

fn default_keep_alive() -> u64 {
    30
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config: {0}")]
    Parse(Box<FigmentError>),

    #[error("invalid config: {0}")]
    Invalid(String),
}

impl From<FigmentError> for ConfigError {
    fn from(e: FigmentError) -> Self {
        Self::Parse(Box::new(e))
    }
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "config file not found"),
        });
    }
    let fig = Figment::new()
        .merge(Toml::file(path))
        .merge(Env::prefixed("ATHENA__").split("__"));

    let cfg: Config = fig.extract()?;

    if cfg.locales.is_empty() {
        return Err(ConfigError::Invalid("`locales` must not be empty".into()));
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_toml_with_skills_section() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let cfg = load(&repo_root.join("athena.example.toml")).expect("example config parses");
        assert_eq!(
            cfg.skills.dir.as_deref(),
            Some(Path::new("/etc/athena-voice/skills"))
        );
        assert!(cfg.skills.per_skill.is_empty());
        assert!(
            !cfg.skills.hot_reload,
            "hot_reload must default off in the shipped example"
        );
    }

    #[test]
    fn parses_per_skill_subtables() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"
locales = ["fr"]
[server]
host = "0.0.0.0"
port = 8080
[storage]
database_url = "sqlite::memory:"
[mqtt]
host = "127.0.0.1"
port = 1883
client_id = "test"
[providers]
stt = "fake"
llm = "fake"
tts = "fake"
[skills]
dir = "/opt/skills"
[skills.weather]
http_allowlist = ["api.example.com"]
config = { units = "metric" }
"#,
        )
        .unwrap();
        let cfg = load(tmp.path()).unwrap();
        assert_eq!(cfg.skills.dir.as_deref(), Some(Path::new("/opt/skills")));
        let weather = cfg
            .skills
            .per_skill
            .get("weather")
            .expect("weather entry present");
        assert_eq!(weather.http_allowlist, vec!["api.example.com".to_string()]);
        assert_eq!(
            weather.config.get("units").map(String::as_str),
            Some("metric")
        );
    }

    #[test]
    fn parses_assist_block_and_defaults() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"
locales = ["fr"]
[server]
host = "0.0.0.0"
port = 8080
[storage]
database_url = "sqlite::memory:"
[mqtt]
host = "127.0.0.1"
port = 1883
client_id = "test"
[providers]
stt = "fake"
llm = "fake"
tts = "fake"
[assist]
enabled = true
"#,
        )
        .unwrap();
        let cfg = load(tmp.path()).unwrap();
        let assist = cfg.assist.expect("assist block parsed");
        assert!(assist.enabled);
        assert_eq!(assist.topic_prefix, "assist");
        assert_eq!(assist.locale.as_str(), "fr");
    }

    #[test]
    fn missing_assist_block_is_none() {
        // athena.example.toml has no [assist] block.
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let cfg = load(&repo_root.join("athena.example.toml")).expect("example parses");
        assert!(cfg.assist.is_none());
    }

    #[test]
    fn parses_assist_profile_toml() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let cfg = load(&repo_root.join("athena.assist.toml")).expect("assist profile parses");
        assert!(cfg.assist.is_some());
        let assist = cfg.assist.unwrap();
        assert!(assist.enabled);
        assert_eq!(assist.topic_prefix, "assist");
        assert_eq!(assist.locale.as_str(), "fr");
    }
}
