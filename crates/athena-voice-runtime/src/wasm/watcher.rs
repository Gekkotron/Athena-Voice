//! Filesystem watcher for the skills directory.
//!
//! Task B (hot-reload) wraps `notify` behind `notify-debouncer-full` with a
//! ~250 ms debounce so a burst of write events on the same `*.wasm` file
//! collapses into a single `Modified`. The debouncer runs on its own OS
//! thread (owned by `notify-debouncer-full`); the tokio-side receiver only
//! sees compact [`WatchEvent`]s over an internal `mpsc`.
//!
//! Only paths with a `.wasm` extension are surfaced — text editors churn
//! swap files (`.wasm.swp`, `.wasm.tmp~`, editor journals) in the same
//! directory and we don't want those to trigger reloads.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Config, PollWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, RecommendedCache, new_debouncer_opt};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::warn;

/// Debounce window applied to raw filesystem events before the runtime sees
/// them. 250 ms is enough to swallow the write/rename/chmod dance most
/// editors perform on save without introducing a perceptible edit-to-reload
/// delay.
const DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKind {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchKind,
}

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("notify watcher setup failed: {0}")]
    Notify(#[from] notify::Error),
}

/// Handle to a running watcher. Dropping it stops the underlying
/// debouncer thread.
pub struct WatcherHandle {
    // Kept alive for its `Drop` — the debouncer's inner thread stops when
    // this value is dropped. Type is erased through a boxed trait object so
    // the caller doesn't have to spell out the generic parameters.
    _debouncer: Box<dyn Send + Sync>,
}

/// Spawn a debounced watcher on `dir`. Returns a handle that must be kept
/// alive for the watcher to remain active and a receiver over which
/// [`WatchEvent`]s stream.
///
/// The debouncer is non-recursive: only immediate children of `dir` are
/// watched. That matches the skill layout (one `.wasm` per skill directly
/// under `dir`).
pub fn spawn_watcher(
    dir: &Path,
) -> Result<(WatcherHandle, mpsc::UnboundedReceiver<WatchEvent>), WatcherError> {
    let (tx, rx) = mpsc::unbounded_channel::<WatchEvent>();
    // Backed by `notify::PollWatcher` rather than the platform native
    // backend (FSEvents / inotify / ReadDirectoryChangesW). Rationale:
    // FSEvents on macOS aggressively coalesces successive events on the
    // same path and often reports only the first flag (Create) for a
    // create+modify+remove burst — even with a debouncer we lose the
    // distinct Modify / Remove signals we need to distinguish a reload
    // from an uninstall. Polling every 100 ms is deterministic across
    // platforms, cheap for the tiny "one wasm per skill" directory, and
    // covers editor-atomic saves (write to temp + rename) equally well.
    let poll_config = Config::default().with_poll_interval(Duration::from_millis(100));
    let mut debouncer = new_debouncer_opt::<_, PollWatcher, RecommendedCache>(
        DEBOUNCE,
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                for ev in events {
                    for wev in classify(&ev.event) {
                        let _ = tx.send(wev);
                    }
                }
            }
            Err(errors) => {
                for e in errors {
                    warn!(error = %e, "filesystem watcher error");
                }
            }
        },
        RecommendedCache::new(),
        poll_config,
    )?;
    debouncer.watch(dir, RecursiveMode::NonRecursive)?;
    Ok((
        WatcherHandle {
            _debouncer: Box::new(debouncer),
        },
        rx,
    ))
}

fn classify(event: &notify::Event) -> Vec<WatchEvent> {
    use notify::EventKind;
    let mut out = Vec::new();
    for path in &event.paths {
        if path.extension().and_then(|s| s.to_str()) != Some("wasm") {
            continue;
        }
        let kind = match event.kind {
            EventKind::Create(_) => Some(WatchKind::Added),
            EventKind::Modify(_) => {
                // Rename operations show up as `Modify(Name(_))`; `notify`
                // reports the from/to sides separately with the path we can
                // interpret as add-or-remove based on existence.
                if path.exists() {
                    Some(WatchKind::Modified)
                } else {
                    Some(WatchKind::Removed)
                }
            }
            EventKind::Remove(_) => Some(WatchKind::Removed),
            _ => None,
        };
        if let Some(kind) = kind {
            out.push(WatchEvent {
                path: path.clone(),
                kind,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    /// A watcher run against a tempdir sees an Added / Modified / Removed
    /// WatchEvent for the target file within the per-phase window.
    ///
    /// The debouncer coalesces bursts within the 250 ms window, so we sleep
    /// briefly between phases to avoid an add+remove getting merged into a
    /// single event.
    #[test]
    fn tempdir_watcher_yields_add_modify_remove_events() {
        let dir = tempfile::Builder::new()
            .prefix("athena-watch")
            .tempdir_in("/tmp")
            .unwrap();
        let (_handle, mut rx) = spawn_watcher(dir.path()).expect("watcher");

        // Arm-check: the PollWatcher's baseline snapshot runs on its own
        // thread, and under heavy machine load it can complete AFTER our
        // first write — the file then lands in the baseline and no Added is
        // ever reported. Keep creating fresh probe files until one is seen;
        // only then is the watcher provably live.
        let mut armed = false;
        for i in 0..40 {
            let probe = dir.path().join(format!("probe-{i}.wasm"));
            std::fs::write(&probe, b"x").unwrap();
            if wait_for(
                &mut rx,
                WatchKind::Added,
                &probe,
                Duration::from_millis(500),
            )
            .is_some()
            {
                armed = true;
                break;
            }
        }
        assert!(armed, "watcher never armed within probe budget");

        let path = dir.path().join("smoke.wasm");

        // Phase 1: create.
        std::fs::write(&path, b"hello").unwrap();
        let added = wait_for(
            &mut rx,
            WatchKind::Added,
            &path,
            Duration::from_millis(15_000),
        );
        assert!(added.is_some(), "no Added event within window");

        // Ensure the debounce window closes before the next mutation, so
        // Modified doesn't get coalesced back into the previous batch.
        std::thread::sleep(Duration::from_millis(300));

        // Phase 2: modify.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(b" world").unwrap();
            f.sync_all().unwrap();
        }
        let modified = wait_for(
            &mut rx,
            WatchKind::Modified,
            &path,
            Duration::from_millis(15_000),
        );
        assert!(modified.is_some(), "no Modified event within window");

        std::thread::sleep(Duration::from_millis(300));

        // Phase 3: remove.
        std::fs::remove_file(&path).unwrap();
        let removed = wait_for(
            &mut rx,
            WatchKind::Removed,
            &path,
            Duration::from_millis(15_000),
        );
        assert!(removed.is_some(), "no Removed event within window");
    }

    fn wait_for(
        rx: &mut mpsc::UnboundedReceiver<WatchEvent>,
        kind: WatchKind,
        path: &Path,
        budget: Duration,
    ) -> Option<WatchEvent> {
        let deadline = Instant::now() + budget;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match rx.try_recv() {
                // Compare file names: the watcher may report canonicalized
                // paths (/private/tmp/…) for files created via /tmp.
                Ok(ev) if ev.kind == kind && ev.path.file_name() == path.file_name() => {
                    return Some(ev);
                }
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => return None,
            }
        }
    }
}
