//! Placeholder; fills in Task 8.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("shutdown")]
    Shutdown,
}
