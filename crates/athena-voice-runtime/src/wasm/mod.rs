//! Extism WASM host for Athena-Voice skills.
//!
//! Structure:
//! - `host_fns`   — Extism host function implementations (log, config_get,
//!   state_get/set, mqtt_publish, http_get_json).
//! - `registry`   — `SkillRegistry`: loads `*.wasm` files, caches Extism `Plugin`s,
//!   populates the `RuleIndex` by calling each skill's exported `pattern_rules`.
//! - `dispatcher` — `SkillDispatcher` actor that receives `(session_id, intent)`
//!   and calls into a plugin via `spawn_blocking`.
//! - `watcher`    — Task B: debounced filesystem watcher on the skills dir.
//! - [`spawn_hot_reload_task`] — bridges watcher events into the registry.

pub mod dispatcher;
pub mod error;
pub mod host_fns;
pub mod registry;
pub mod watcher;

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::wasm::registry::{SkillDeps, SkillRegistry};
use crate::wasm::watcher::{WatchEvent, WatchKind};

/// Bridge the watcher's [`WatchEvent`] stream into the [`SkillRegistry`].
///
/// - `Added` / `Modified` → `registry.reload_path(path, &deps)`.
/// - `Removed` → `registry.remove(name)` (name derived from file stem).
///
/// Reload errors are already logged + turned into `Event::SkillReloadFailed`
/// by `reload_path`, so the task itself only forwards + traces.
pub fn spawn_hot_reload_task(
    mut watcher_rx: mpsc::UnboundedReceiver<WatchEvent>,
    registry: Arc<SkillRegistry>,
    deps: SkillDeps,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                maybe = watcher_rx.recv() => match maybe {
                    Some(ev) => handle_event(&registry, &deps, ev),
                    None => break,
                },
            }
        }
        debug!("skill hot-reload task shutting down");
    })
}

fn handle_event(registry: &SkillRegistry, deps: &SkillDeps, ev: WatchEvent) {
    match ev.kind {
        WatchKind::Added | WatchKind::Modified => {
            if let Err(err) = registry.reload_path(&ev.path, deps) {
                // reload_path already emitted SkillReloadFailed; log at
                // debug to avoid double-warning on the same reason.
                debug!(path = %ev.path.display(), error = %err, "reload_path error");
            }
        }
        WatchKind::Removed => {
            let Some(name) = ev.path.file_stem().and_then(|s| s.to_str()) else {
                warn!(path = %ev.path.display(), "watcher path had no file stem");
                return;
            };
            if !registry.remove(name) {
                debug!(skill = %name, "removed event for skill not in registry");
            }
        }
    }
}
