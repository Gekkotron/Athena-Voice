use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

static INIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("tracing subscriber already initialised")]
    AlreadyInit,

    #[error("failed to install subscriber: {0}")]
    Install(String),
}

pub fn init() -> Result<(), LoggingError> {
    if INIT.swap(true, Ordering::SeqCst) {
        return Err(LoggingError::AlreadyInit);
    }
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let json = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true);
    tracing_subscriber::registry()
        .with(filter)
        .with(json)
        .try_init()
        .map_err(|e| LoggingError::Install(e.to_string()))?;
    Ok(())
}
