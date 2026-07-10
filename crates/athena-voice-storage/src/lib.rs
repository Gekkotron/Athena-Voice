#![deny(warnings)]
//! Persistence layer for Athena-Voice.

pub mod error;
pub mod models;
pub mod store;

pub use error::StoreError;
pub use store::Store;
