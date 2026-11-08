//! Noticing new status files: a watcher on the run folder with a 150 ms settle time per file, and a
//! scan every 20 s that catches what the watcher missed (`start` and `schedule` of
//! `src/main/usage/index.ts`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use mc_core::usage::tap::STATUS_SUFFIX;
use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Core;

const SETTLE: Duration = Duration::from_millis(150);
pub(crate) const SCAN_INTERVAL: Duration = Duration::from_secs(20);

/// Watches `dir` and reads a status file once it has been quiet for 150 ms. None when the folder
/// cannot be watched; the scan still picks changes up then.
pub(crate) fn watch(core: &Arc<Core>, dir: &Path) -> Option<(RecommendedWatcher, JoinHandle<()>)> {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let handler = move |res: notify::Result<Event>| {
        let Ok(event) = res else { return };
        // our own reads must not come back as changes
        if matches!(event.kind, EventKind::Access(kind) if kind != AccessKind::Close(AccessMode::Write)) {
            return;
        }
        for path in &event.paths {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()).filter(|n| n.ends_with(STATUS_SUFFIX)) {
                let _ = tx.send(name.to_string());
            }
        }
    };
    let mut watcher = match notify::recommended_watcher(handler) {
        Ok(watcher) => watcher,
        Err(e) => {
            log::warn!("usage: cannot watch {}: {e}", dir.display());
            return None;
        }
    };
    if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        log::warn!("usage: cannot watch {}: {e}", dir.display());
        return None;
    }
    Some((watcher, async_runtime::spawn(settle(Arc::downgrade(core), rx))))
}

/// Reads each file named on `rx` once no further change has come for it for [`SETTLE`]. Ends when
/// the watcher is dropped or the service is gone.
async fn settle(core: Weak<Core>, mut rx: mpsc::UnboundedReceiver<String>) {
    let mut due: HashMap<String, Instant> = HashMap::new();
    loop {
        let next = due.values().min().copied();
        tokio::select! {
            name = rx.recv() => match name {
                Some(name) => {
                    due.insert(name, Instant::now() + SETTLE);
                }
                None => return,
            },
            _ = tokio::time::sleep_until(next.unwrap_or_else(Instant::now)), if next.is_some() => {
                let now = Instant::now();
                let ready: Vec<String> = due.iter().filter(|(_, at)| **at <= now).map(|(name, _)| name.clone()).collect();
                let Some(core) = core.upgrade() else { return };
                for name in ready {
                    due.remove(&name);
                    core.ingest(&name);
                }
            }
        }
    }
}

/// Scans the run folder every [`SCAN_INTERVAL`].
pub(crate) fn scan_periodically(core: &Arc<Core>) -> JoinHandle<()> {
    let core = Arc::downgrade(core);
    async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(SCAN_INTERVAL).await;
            let Some(core) = core.upgrade() else { return };
            core.scan();
        }
    })
}
