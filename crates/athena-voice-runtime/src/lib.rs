#![deny(warnings)]
//! Athena-Voice runtime: actor DAG + MQTT satellite adapter + event bus.

pub mod config;
pub mod error;
pub mod event_bus;
pub mod intent;
pub mod locale;
pub mod mqtt;
pub mod pipeline;
pub mod satellite;
pub mod session;
pub mod wasm;

pub use error::RuntimeError;

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_providers::ProviderFactory;

use crate::event_bus::EventBus;
use crate::mqtt::{MqttClient, MqttConfig};
use crate::satellite::{SatelliteDeps, spawn_satellite};
use crate::session::SessionManager;

/// Top-level runtime handle. Constructed via `Runtime::spawn`. Drop to abort
/// all background tasks — or call `shutdown()` for a clean drain.
pub struct Runtime {
    pub shutdown: CancellationToken,
    pub sessions: Arc<SessionManager>,
    pub event_bus: Arc<EventBus>,
    satellite_task: JoinHandle<()>,
    mqtt_pump_task: Option<JoinHandle<()>>,
}

impl Runtime {
    /// Spawns the runtime: connects MQTT (queued), subscribes, and spawns the
    /// SatelliteAdapter. The event loop pump is included via `spawn_satellite`.
    pub fn spawn(
        mqtt_cfg: MqttConfig,
        factory: Arc<ProviderFactory>,
    ) -> Result<Self, RuntimeError> {
        let client = MqttClient::connect(mqtt_cfg)?;
        let sessions = Arc::new(SessionManager::default());
        let event_bus = Arc::new(EventBus::new(1024));
        let shutdown = CancellationToken::new();

        // Empty rule index until Plan 4 Task 6 (SkillRegistry) populates it.
        let matcher = Arc::new(intent::IntentMatcher::new());
        let rules = Arc::new(ArcSwap::from_pointee(intent::RuleIndex::new()));

        let deps = SatelliteDeps {
            mqtt: client.tx.clone(),
            event_loop: client.event_loop.clone(),
            factory,
            session_manager: sessions.clone(),
            event_bus: event_bus.sender(),
            matcher,
            rules,
            dispatcher: None,
            shutdown: shutdown.clone(),
        };
        let satellite_task = spawn_satellite(deps);

        // Also start the MQTT event mirror task so athena/events/* is populated.
        let mirror = event_bus::spawn_mqtt_mirror(event_bus.sender(), client.tx);

        Ok(Self {
            shutdown,
            sessions,
            event_bus,
            satellite_task,
            mqtt_pump_task: Some(mirror),
        })
    }

    /// Cleanly shuts the runtime down: cancels the shutdown token and awaits
    /// the satellite adapter's join handle.
    pub async fn shutdown(mut self) {
        self.shutdown.cancel();
        self.sessions.cancel_all();
        let _ = self.satellite_task.await;
        if let Some(h) = self.mqtt_pump_task.take() {
            h.abort();
        }
    }
}
