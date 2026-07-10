#![deny(warnings)]
//! Athena-Voice runtime: actor DAG + MQTT satellite adapter + event bus.

pub mod config;
pub mod error;
pub mod event_bus;
pub mod locale;
pub mod mqtt;
pub mod pipeline;
pub mod satellite;
pub mod session;

pub use error::RuntimeError;
