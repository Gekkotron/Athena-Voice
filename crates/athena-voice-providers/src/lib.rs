#![deny(warnings)]
//! Provider adapters for Athena-Voice. Plan 2 ships fakes only; real providers land in Plan 3.

pub mod circuit;
pub mod error;
pub mod factory;
pub mod no_llm;
pub mod remote;
pub mod retry;
pub mod testing;

pub use error::{LlmError, SttError, TtsError};
pub use factory::{ProviderConfig, ProviderFactory, StageChoice};
