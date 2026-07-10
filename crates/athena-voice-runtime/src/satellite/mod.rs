//! SatelliteAdapter: bidirectional MQTT ↔ pipeline bridge.

pub mod ingress;

pub use ingress::{SatelliteDeps, spawn_satellite};
