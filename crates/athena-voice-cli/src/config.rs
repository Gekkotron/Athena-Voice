use std::path::Path;

use figment::providers::{Env, Format, Toml};
use figment::{Error as FigmentError, Figment};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use athena_voice_core::ids::Locale;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub locales: Vec<Locale>,
    pub server: ServerConfig,
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub database_url: String,
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
