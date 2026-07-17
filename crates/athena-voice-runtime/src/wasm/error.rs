//! Errors raised by host functions before they yield control to WASM.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostFnError {
    #[error("http url could not be parsed: {0}")]
    HttpBadUrl(String),
    #[error("http scheme not allowed: {0}")]
    HttpBadScheme(String),
    #[error("http host not on allowlist: {0}")]
    HttpHostNotAllowed(String),
    #[error("http request failed: {0}")]
    HttpFailed(String),
}
