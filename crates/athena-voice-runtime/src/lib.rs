#![deny(warnings)]
//! Athena-Voice runtime: actor DAG + MQTT satellite adapter + event bus.

pub mod config;
pub mod error;
pub mod event_bus;
pub mod audio;
pub mod intent;
pub mod locale;
pub mod mqtt;
pub mod pipeline;
pub mod satellite;
pub mod session;
pub mod wasm;

pub use error::RuntimeError;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use athena_voice_providers::ProviderFactory;
use athena_voice_storage::Store;

use crate::event_bus::EventBus;
use crate::mqtt::{MqttClient, MqttConfig};
use crate::satellite::{SatelliteDeps, spawn_satellite};
use crate::session::SessionManager;
use crate::wasm::dispatcher::SkillDispatcher;
use crate::wasm::registry::{SkillConfig, SkillDeps, SkillRegistry};

/// WASM skill loading parameters for [`Runtime::spawn`]. `None` (or a
/// missing `dir`) starts the runtime with zero skills — every matched
/// intent then falls back to the LLM.
pub struct SkillsInit {
    /// Directory scanned for `*.wasm` skills at startup.
    pub dir: PathBuf,
    /// Storage backend shared with the rest of the runtime.
    pub store: Arc<dyn Store>,
    /// Locales for which each skill's `pattern_rules(locale)` is queried.
    pub locales: Vec<String>,
    /// Per-skill config keyed by skill name (wasm file stem).
    pub per_skill: HashMap<String, SkillConfig>,
}

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
        skills: Option<SkillsInit>,
        session_idle: std::time::Duration,
    ) -> Result<Self, RuntimeError> {
        let client = MqttClient::connect(mqtt_cfg)?;
        let sessions = Arc::new(SessionManager::default());
        let event_bus = Arc::new(EventBus::new(1024));
        let shutdown = CancellationToken::new();

        let matcher = Arc::new(intent::IntentMatcher::new());

        // Load WASM skills when a skills dir is configured; otherwise start
        // with an empty rule index and no dispatcher (pure LLM fallback).
        let (rules, dispatcher) = match skills.filter(|init| init.dir.is_dir()) {
            Some(init) => {
                let deps = SkillDeps {
                    store: init.store,
                    mqtt: client.tx.clone(),
                    tokio: tokio::runtime::Handle::current(),
                    http: reqwest::Client::new(),
                    locales: init.locales,
                    per_skill: init.per_skill,
                    event_tx: Some(event_bus.sender()),
                    audio_event_tx: event_bus.sender(),
                };
                let registry = Arc::new(
                    SkillRegistry::load_dir(&init.dir, &deps)
                        .map_err(|e| RuntimeError::Config(format!("skill load: {e}")))?,
                );
                tracing::info!(
                    dir = %init.dir.display(),
                    skills = ?registry.skill_names(),
                    "skills loaded"
                );
                let rules = registry.patterns_handle();
                let (handle, _task) =
                    SkillDispatcher::spawn(registry, event_bus.sender(), shutdown.clone());
                (rules, Some(handle))
            }
            None => (
                Arc::new(ArcSwap::from_pointee(intent::RuleIndex::new())),
                None,
            ),
        };

        let deps = SatelliteDeps {
            mqtt: client.tx.clone(),
            event_loop: client.event_loop.clone(),
            factory,
            session_manager: sessions.clone(),
            event_bus: event_bus.sender(),
            matcher,
            rules,
            dispatcher,
            shutdown: shutdown.clone(),
        };
        let satellite_task = spawn_satellite(deps);

        // Also start the MQTT event mirror task so athena/events/* is populated.
        let mirror = event_bus::spawn_mqtt_mirror(event_bus.sender(), client.tx);

        // Reap sessions whose satellite went silent without sending `end` —
        // their DAG actors would otherwise stay parked until shutdown.
        {
            let sessions = sessions.clone();
            let cancel = shutdown.clone();
            drop(tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
                loop {
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        _ = tick.tick() => {
                            for sid in sessions.reap_idle(session_idle) {
                                tracing::info!(session = %sid, "reaped idle session");
                            }
                        }
                    }
                }
            }));
        }

        // Conditionally start audio sink
        #[cfg(feature = "audio")]
        {
            let event_bus_clone = event_bus.clone();
            drop(tokio::spawn(async move {
                if let Err(e) = audio::AudioSink::new(event_bus_clone.subscribe()).run().await {
                    tracing::error!("audio sink failed: {e}");
                }
            }));
        }

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
    // Audio sink task runs until events end or failure.
}
}
