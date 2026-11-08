//! Usage numbers of the accounts: `src/main/usage/index.ts`, the queue of `src/main/usage/probe.ts`,
//! and the usage wiring of `src/main/index.ts` (alert delivery, the 30 s reset check, the status
//! line tap of a session's settings file).
//!
//! Live numbers come from the status files that the status line tap of running sessions writes
//! into `<root>/run`; `/usage` probes add the server's answer. Every change is sent to the window
//! as `usage:changed`, saved to `usage-cache.json` and run through the limit alerts.
//!
//! Integration: `set_deps`, `set_notifier` and `set_session_meta_sink` once, `prune` + `start` at
//! boot, `start_prober` once claude has been located and the accounts checked, `stop` on quit.
//! Sessions use `status_line_setting` / `files_for` when they write their settings file and
//! `session_ended` when they end, then `request_refresh` for the account (its last requests are not
//! in the status line data yet). The window's focus calls `window_focused`.

mod bash;
mod cache;
mod clock;
mod prober;
mod watch;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mc_core::types::{UsageCache, UsageState};
use mc_core::usage::tap::{self, parse_status_file_name, settings_file_name, status_file_name, STATUS_SUFFIX};
use mc_core::usage::{
    alert_message, merge_usage, normalize, parse_status, AlertEvent, AlertMessage, LimitAlerts, LocalClock, ProbeResult, SessionMeta,
    TickResult,
};
use notify::RecommendedWatcher;
use serde_json::Value;
use tauri::async_runtime::{self, JoinHandle};
use tauri::AppHandle;

pub use bash::git_bash_path;
pub use prober::{ProbeDeps, ProbeProfile};

use prober::{ProbeRun, Prober};

/// Sessions clean up after themselves; this catches files left behind by a crash or an external terminal.
const STALE_FILE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const SAVE_DELAY: Duration = Duration::from_secs(1);
const LIMIT_RESET_CHECK: Duration = Duration::from_secs(30);
/// Right at its reset time `/usage` may still report the old window.
const REFRESH_AFTER_RESET: Duration = Duration::from_secs(90);
/// More time than this between two reset checks means the computer slept (or the clock jumped).
const CLOCK_JUMP_MS: i64 = 90_000;
const USAGE_REFRESH_ON_FOCUS_MS: i64 = 5 * 60_000;

/// Delivers what happened to one account's limits: (profile id, message). It decides between a
/// line in the window and a system notification, and honours the `limitAlerts` setting.
pub type Notifier = Arc<dyn Fn(&str, AlertMessage) + Send + Sync>;
/// Takes what a status file says about its session: (the app's session id, what it says).
pub type MetaSink = Arc<dyn Fn(&str, SessionMeta) + Send + Sync>;
type Emitter = Box<dyn Fn(&UsageCache) + Send + Sync>;

/// The files of one session in the run folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFiles {
    /// The settings file the session is started with (`--settings`).
    pub settings_file: PathBuf,
    /// Where its status line tap writes.
    pub status_file: PathBuf,
}

/// Rate limit usage per account.
pub struct UsageTracker {
    core: Arc<Core>,
}

pub(crate) struct Core {
    run_dir: PathBuf,
    probe_dir: PathBuf,
    cache_file: PathBuf,
    state: Mutex<UsageCache>,
    /// Status file name to the modification time it had when last read.
    mtimes: Mutex<HashMap<String, SystemTime>>,
    save_scheduled: AtomicBool,
    alerts: Mutex<LimitAlerts>,
    notifier: RwLock<Option<Notifier>>,
    meta_sink: RwLock<Option<MetaSink>>,
    emit: Emitter,
    prober: Prober,
    started: AtomicBool,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    tap_ok: OnceLock<bool>,
}

impl UsageTracker {
    /// Reads the cache; nothing runs until [`start`](Self::start).
    pub fn new(app: AppHandle, root: PathBuf) -> Self {
        let emit = move |all: &UsageCache| crate::events::emit(&app, crate::events::USAGE_CHANGED, all.clone());
        Self::build(root, Box::new(emit), prober::real_run())
    }

    fn build(root: PathBuf, emit: Emitter, run: ProbeRun) -> Self {
        let run_dir = root.join("run");
        let probe_dir = root.join("probe");
        let cache_file = root.join("usage-cache.json");
        let state = cache::load(&cache_file);
        let core = Arc::new_cyclic(|weak: &Weak<Core>| {
            let weak = weak.clone();
            let apply = move |profile_id: &str, result: ProbeResult| {
                if let Some(core) = weak.upgrade() {
                    core.apply_probe(profile_id, &result);
                }
            };
            Core {
                prober: Prober::new(probe_dir.clone(), Arc::new(clock::SystemClock), Box::new(apply), run),
                run_dir,
                probe_dir,
                cache_file,
                state: Mutex::new(state),
                mtimes: Mutex::default(),
                save_scheduled: AtomicBool::new(false),
                alerts: Mutex::default(),
                notifier: RwLock::new(None),
                meta_sink: RwLock::new(None),
                emit,
                started: AtomicBool::new(false),
                tasks: Mutex::default(),
                watcher: Mutex::new(None),
                tap_ok: OnceLock::new(),
            }
        });
        UsageTracker { core }
    }

    // ---------- wiring ----------

    /// The accounts, the claude binary and the environment to probe with.
    pub fn set_deps(&self, deps: Arc<dyn ProbeDeps>) {
        self.core.prober.set_deps(deps);
    }

    pub fn set_notifier(&self, notify: impl Fn(&str, AlertMessage) + Send + Sync + 'static) {
        *write(&self.core.notifier) = Some(Arc::new(notify));
    }

    pub fn set_session_meta_sink(&self, sink: impl Fn(&str, SessionMeta) + Send + Sync + 'static) {
        *write(&self.core.meta_sink) = Some(Arc::new(sink));
    }

    /// The time zone `/usage` reset times are read in. By default the current offset of the
    /// Windows time zone, or UTC elsewhere.
    pub fn set_clock(&self, clock: Arc<dyn LocalClock>) {
        self.core.prober.set_clock(clock);
    }

    // ---------- running ----------

    /// Purges old files from the run folder, starts watching it, and checks every 30 s whether a
    /// limit has reset. Only the first call does anything.
    pub fn start(&self) {
        let core = &self.core;
        if core.started.swap(true, Ordering::SeqCst) {
            return;
        }
        for dir in [&core.run_dir, &core.probe_dir] {
            if let Err(e) = fs::create_dir_all(dir) {
                log::warn!("usage: cannot create {}: {e}", dir.display());
            }
        }
        core.purge_stale();
        let mut tasks = Vec::new();
        if let Some((watcher, task)) = watch::watch(core, &core.run_dir) {
            *lock(&core.watcher) = Some(watcher);
            tasks.push(task);
        }
        tasks.push(watch::scan_periodically(core));
        tasks.push(limit_reset_checks(core));
        lock(&core.tasks).extend(tasks);
        core.scan();
    }

    /// A first `/usage` round now, then one every 10 minutes.
    pub fn start_prober(&self) {
        self.core.prober.start();
    }

    /// Stops watching and probing (waiting refreshes resolve with false) and saves the cache.
    pub fn stop(&self) {
        let core = &self.core;
        core.prober.stop();
        lock(&core.watcher).take();
        for task in lock(&core.tasks).drain(..) {
            task.abort();
        }
        if core.save_scheduled.load(Ordering::SeqCst) {
            core.save_now();
        }
    }

    // ---------- numbers ----------

    /// Profile id to usage: the `usage` of the window's state and of `usage:changed`.
    pub fn snapshot(&self) -> UsageCache {
        lock(&self.core.state).clone()
    }

    pub fn usage_of(&self, profile_id: &str) -> Option<UsageState> {
        lock(&self.core.state).get(profile_id).cloned()
    }

    /// Drops the usage of accounts that no longer exist.
    pub fn prune<I, S>(&self, valid_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let keep: HashSet<String> = valid_ids.into_iter().map(|id| id.as_ref().to_string()).collect();
        let gone: Vec<String> = {
            let mut state = lock(&self.core.state);
            let gone: Vec<String> = state.keys().filter(|id| !keep.contains(*id)).cloned().collect();
            for id in &gone {
                state.remove(id);
            }
            gone
        };
        if gone.is_empty() {
            return;
        }
        let mut alerts = lock(&self.core.alerts);
        for id in &gone {
            alerts.forget(id);
        }
        drop(alerts);
        self.core.save_now();
    }

    /// Takes what `/usage` reported for an account.
    pub fn apply_probe(&self, profile_id: &str, result: &ProbeResult) {
        self.core.apply_probe(profile_id, result);
    }

    // ---------- probing ----------

    /// Asks `/usage` for the account (queued at once; one account at a time). Resolves with true
    /// once fresh numbers have been applied, false when there were none or the service stopped.
    pub fn refresh(&self, profile_id: &str) -> impl Future<Output = bool> + Send + 'static {
        self.core.prober.refresh(profile_id)
    }

    /// [`refresh`](Self::refresh) without waiting for the answer.
    pub fn request_refresh(&self, profile_id: &str) {
        drop(self.core.prober.refresh(profile_id));
    }

    pub fn refresh_all(&self) {
        self.core.prober.refresh_all();
    }

    pub fn refresh_all_if_older_than(&self, ms: i64) {
        self.core.prober.refresh_all_if_older_than(ms);
    }

    /// The window came to the front: numbers older than 5 minutes are fetched again.
    pub fn window_focused(&self) {
        self.refresh_all_if_older_than(USAGE_REFRESH_ON_FOCUS_MS);
    }

    /// The reset check that otherwise runs every 30 s; for a power-resume signal, should one be wired.
    pub fn check_limit_resets(&self) {
        self.core.check_limit_resets();
    }

    // ---------- sessions ----------

    pub fn files_for(&self, profile_id: &str, session_id: &str) -> SessionFiles {
        SessionFiles {
            settings_file: self.core.run_dir.join(settings_file_name(profile_id, session_id)),
            status_file: self.core.run_dir.join(status_file_name(profile_id, session_id)),
        }
    }

    /// Whether status lines run in a POSIX shell here, which the tap needs. Found once.
    pub fn tap_supported(&self) -> bool {
        *self.core.tap_ok.get_or_init(|| git_bash_path().is_some())
    }

    /// `{ "statusLine": { ... } }` to merge into the settings file a session starts with: the tap
    /// in front of the status line the session would have had (`config_dir` is the account's
    /// effective config folder). None when the tap cannot run here.
    pub fn status_line_setting(&self, cwd: &Path, config_dir: &Path, profile_id: &str, session_id: &str, profile_name: &str) -> Option<Value> {
        if !self.tap_supported() {
            return None;
        }
        let inner = tap::find_inner_status_line(cwd, config_dir, |file| fs::read(file).ok().map(|b| String::from_utf8_lossy(&b).into_owned()));
        let status_file = self.files_for(profile_id, session_id).status_file;
        Some(tap::tap_settings(&status_file.to_string_lossy(), inner.as_ref(), profile_name))
    }

    /// A session ended: takes the last sample of its status file, then removes its settings file,
    /// its status file and a status file left half-written.
    pub fn session_ended(&self, profile_id: &str, session_id: &str) {
        let name = status_file_name(profile_id, session_id);
        self.core.ingest(&name);
        let files = self.files_for(profile_id, session_id);
        let mut tmp = files.status_file.clone().into_os_string();
        tmp.push(".tmp");
        for file in [files.settings_file, files.status_file, PathBuf::from(tmp)] {
            let _ = fs::remove_file(file);
        }
        lock(&self.core.mtimes).remove(&name);
    }
}

impl Core {
    fn apply_probe(self: &Arc<Self>, profile_id: &str, result: &ProbeResult) {
        let now = now_ms();
        self.update(profile_id, |prev| Some(normalize::apply_probe(prev, result, now)));
    }

    /// Reads one status file of the run folder.
    fn ingest(self: &Arc<Self>, name: &str) {
        let file = self.run_dir.join(name);
        let Ok(meta) = fs::metadata(&file) else { return };
        lock(&self.mtimes).insert(name.to_string(), meta.modified().unwrap_or(UNIX_EPOCH));
        let Ok(bytes) = fs::read(&file) else { return };
        let Some((profile_id, session_id)) = parse_status_file_name(name) else { return };
        let status = parse_status(&String::from_utf8_lossy(&bytes));
        if let (Some(meta), Some(sink)) = (status.meta, read(&self.meta_sink).clone()) {
            sink(session_id, meta);
        }
        let Some(sample) = status.usage else { return };
        self.update(profile_id, |prev| merge_usage(prev, &sample, now_ms()));
    }

    /// Replaces the usage of an account when `next` returns something: reports it, runs it through
    /// the alerts and saves it soon.
    fn update(self: &Arc<Self>, profile_id: &str, next: impl FnOnce(Option<&UsageState>) -> Option<UsageState>) {
        let events = {
            let mut state = lock(&self.state);
            let Some(next) = next(state.get(profile_id)) else { return };
            let events = lock(&self.alerts).observe(profile_id, &next, now_ms());
            state.insert(profile_id.to_string(), next);
            // under the lock, so that the window gets the snapshots in order
            (self.emit)(&state);
            events
        };
        self.deliver(events);
        self.schedule_save();
    }

    /// Tells the user about limits that passed a threshold or started over: one message per account.
    fn deliver(&self, events: Vec<AlertEvent>) {
        if events.is_empty() {
            return;
        }
        let (Some(notify), Some(deps)) = (read(&self.notifier).clone(), self.prober.deps()) else { return };
        let mut by_profile: Vec<(String, Vec<AlertEvent>)> = Vec::new();
        for event in events {
            match by_profile.iter_mut().find(|(id, _)| id == event.profile_id()) {
                Some((_, list)) => list.push(event),
                None => by_profile.push((event.profile_id().to_string(), vec![event])),
            }
        }
        let profiles = deps.profiles();
        let now = now_ms();
        for (profile_id, list) in by_profile {
            let Some(profile) = profiles.iter().find(|p| p.id == profile_id) else { continue };
            if let Some(message) = alert_message(&profile.name, &list, now) {
                notify(&profile_id, message);
            }
        }
    }

    fn check_limit_resets(self: &Arc<Self>) {
        let TickResult { events, refresh } = lock(&self.alerts).tick(now_ms());
        self.deliver(events);
        for profile_id in refresh {
            let core = Arc::downgrade(self);
            async_runtime::spawn(async move {
                tokio::time::sleep(REFRESH_AFTER_RESET).await;
                if let Some(core) = core.upgrade() {
                    drop(core.prober.refresh(&profile_id));
                }
            });
        }
    }

    /// Reads every status file that changed since it was last read.
    fn scan(self: &Arc<Self>) {
        let Ok(entries) = fs::read_dir(&self.run_dir) else { return };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if !name.ends_with(STATUS_SUFFIX) {
                continue;
            }
            // removed between the listing and here
            let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else { continue };
            let known = lock(&self.mtimes).get(&name) == Some(&mtime);
            if !known {
                self.ingest(&name);
            }
        }
    }

    fn purge_stale(&self) {
        let Some(cutoff) = SystemTime::now().checked_sub(Duration::from_millis(STALE_FILE_MS)) else { return };
        let Ok(entries) = fs::read_dir(&self.run_dir) else { return };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() && meta.modified().is_ok_and(|at| at < cutoff) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn schedule_save(self: &Arc<Self>) {
        if self.save_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let core = Arc::downgrade(self);
        async_runtime::spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            if let Some(core) = core.upgrade() {
                core.save_now();
            }
        });
    }

    fn save_now(&self) {
        self.save_scheduled.store(false, Ordering::SeqCst);
        let snapshot = lock(&self.state).clone();
        // the cache is only a convenience across restarts
        if let Err(e) = cache::save(&self.cache_file, &snapshot) {
            log::warn!("usage: cannot save {}: {e}", self.cache_file.display());
        }
    }
}

/// Checks every 30 s whether a limit has reset. A longer gap between two checks means the computer
/// slept: a limit may have reset meanwhile, and the numbers are old.
fn limit_reset_checks(core: &Arc<Core>) -> JoinHandle<()> {
    let core = Arc::downgrade(core);
    async_runtime::spawn(async move {
        let mut last = now_ms();
        loop {
            tokio::time::sleep(LIMIT_RESET_CHECK).await;
            let Some(core) = core.upgrade() else { return };
            let now = now_ms();
            let resumed = now - last > CLOCK_JUMP_MS;
            last = now;
            core.check_limit_resets();
            if resumed {
                core.prober.refresh_all_if_older_than(USAGE_REFRESH_ON_FOCUS_MS);
            }
        }
    })
}

/// `Date.now()`.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
