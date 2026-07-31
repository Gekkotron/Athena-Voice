#![deny(warnings)]
//! Athena-Voice runtime: actor DAG + MQTT satellite adapter + event bus.

pub mod assist;
pub mod audio;
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

/// Paths to the `.wasm` skill fixtures this crate's `build.rs` builds for its
/// own integration tests, re-exported so other crates' tests can load a real
/// skill without depending on the gitignored `skills/*.wasm` bundle (those
/// are produced by each `skills-*/build.sh` and aren't present in a fresh
/// clone) or duplicating the wasm32-wasip1 build step. Gated behind the
/// `test-support` feature, enabled only from `[dev-dependencies]`.
#[cfg(feature = "test-support")]
pub mod test_support {
    /// Built from the `skills-smoke-test` crate.
    pub const SMOKE_TEST_WASM: &str = env!("SMOKE_TEST_WASM");
    /// Built from the `skills-jeedom` crate; exports a `config_schema` that
    /// marks `api_key` secret and `base_url` url-typed.
    pub const JEEDOM_TEST_WASM: &str = env!("JEEDOM_TEST_WASM");
}

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
    /// Skills present in `dir` but disabled via the web UI; they are
    /// unloaded right after the directory scan.
    pub disabled: Vec<String>,
}

/// Live handle to the skill layer for the admin API: reload, remove,
/// schema/name queries. `deps.per_skill` is the merged config snapshot from
/// startup; the admin API overrides entries before each reload.
#[derive(Clone)]
pub struct SkillsHandle {
    pub registry: Arc<SkillRegistry>,
    pub deps: SkillDeps,
    pub dir: PathBuf,
}

/// Top-level runtime handle. Constructed via `Runtime::spawn`. Drop to abort
/// all background tasks — or call `shutdown()` for a clean drain.
pub struct Runtime {
    pub shutdown: CancellationToken,
    pub sessions: Arc<SessionManager>,
    pub event_bus: Arc<EventBus>,
    pub skills: Option<SkillsHandle>,
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
        assist: Option<assist::AssistInit>,
        session_idle: std::time::Duration,
    ) -> Result<Self, RuntimeError> {
        let client = MqttClient::connect(mqtt_cfg)?;
        let sessions = Arc::new(SessionManager::default());
        let event_bus = Arc::new(EventBus::new(1024));
        let shutdown = CancellationToken::new();

        let matcher = Arc::new(intent::IntentMatcher::new());

        // Load WASM skills when a skills dir is configured; otherwise start
        // with an empty rule index and no dispatcher (pure LLM fallback).
        let (rules, dispatcher, skills_handle) = match skills.filter(|init| init.dir.is_dir()) {
            Some(init) => {
                let disabled = init.disabled;
                let dir = init.dir.clone();
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
                    SkillRegistry::load_dir(&dir, &deps)
                        .map_err(|e| RuntimeError::Config(format!("skill load: {e}")))?,
                );
                for name in &disabled {
                    if registry.remove(name) {
                        tracing::info!(skill = %name, "skill disabled via settings; unloaded");
                    }
                }
                tracing::info!(
                    dir = %dir.display(),
                    skills = ?registry.skill_names(),
                    "skills loaded"
                );
                let rules = registry.patterns_handle();
                let handle = SkillsHandle {
                    registry: registry.clone(),
                    deps: deps.clone(),
                    dir,
                };
                let (dispatcher_handle, _task) =
                    SkillDispatcher::spawn(registry, event_bus.sender(), shutdown.clone());
                (rules, Some(dispatcher_handle), Some(handle))
            }
            None => (
                Arc::new(ArcSwap::from_pointee(intent::RuleIndex::new())),
                None,
                None,
            ),
        };

        let assist_bridge = assist.map(|init| {
            let bridge = assist::AssistBridge::new(
                init,
                assist::AssistDeps {
                    publisher: Arc::new(wasm::host_fns::AsyncClientPublisher(client.tx.clone())),
                    factory: factory.clone(),
                    matcher: matcher.clone(),
                    rules: rules.clone(),
                    dispatcher: dispatcher.clone(),
                    event_bus: event_bus.sender(),
                    shutdown: shutdown.clone(),
                },
            );
            let wildcard = bridge.transcription_wildcard();
            let mqtt = client.tx.clone();
            drop(tokio::spawn(async move {
                // Queued like the satellite subscribe; rumqttc retries on reconnect.
                if let Err(e) = mqtt.subscribe(wildcard, rumqttc::QoS::AtMostOnce).await {
                    tracing::warn!(error = %e, "assist subscribe failed");
                }
            }));
            bridge
        });

        let deps = SatelliteDeps {
            mqtt: client.tx.clone(),
            event_loop: client.event_loop.clone(),
            factory,
            session_manager: sessions.clone(),
            event_bus: event_bus.sender(),
            matcher,
            rules,
            dispatcher,
            assist: assist_bridge,
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
                if let Err(e) = audio::AudioSink::new(event_bus_clone.subscribe())
                    .run()
                    .await
                {
                    tracing::error!("audio sink failed: {e}");
                }
            }));
        }

        Ok(Self {
            shutdown,
            sessions,
            event_bus,
            skills: skills_handle,
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
