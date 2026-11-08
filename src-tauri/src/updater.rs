//! Updates from GitHub Releases (`tauri-plugin-updater`). The release workflow publishes a signed
//! installer per platform and a `latest.json` that lists them; where that file is and the public
//! key that checks the downloads are `plugins.updater` of `tauri.conf.json`.
//!
//! [`start`] looks for a new version 10 seconds after the start and then once a day, unless
//! Settings → `checkUpdates` is switched off. A version that is found is announced to the window
//! once per run of the app (`updater:available`), and not at all after "Later" ([`dismiss`]). The
//! window's "Check now" is [`check`]. [`install`] downloads the update (`updater:progress`), ends the
//! app as quitting does (the tabs are saved, the sessions end) and installs it; the new version then
//! starts by itself and reopens the tabs.
//!
//! The new version starts without the arguments of this run: `--hidden` (a start with the system)
//! would keep its window closed, and `--open-folder` (the Explorer menu) would open that folder a
//! second time. On Windows the NSIS installer starts it: Tauri's template runs the app after a
//! passive install when it gets `/R`, with what follows `/ARGS`; the updater is told not to add
//! `/R /ARGS <this run's arguments>` itself and gets a bare `/R` instead. On macOS the new bundle is
//! put in place while this version runs, and [`restart`] starts it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use mc_core::types::AppSettings;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

use crate::error::{CmdError, CmdResult};
use crate::events;
use crate::state::{is, lock, AppState};

/// A newer version was found: `{ info: UpdateInfo }`.
pub const UPDATER_AVAILABLE: &str = "updater:available";
/// The download of the update: `{ downloaded, total }` in bytes, `total` null when not known.
pub const UPDATER_PROGRESS: &str = "updater:progress";

/// The window's channels of the updater. They are in `INVOKE_CHANNELS` of `src/shared/ipc.ts`;
/// TODO: add them to mc-core's copy of that list (`mc_core::types::INVOKE_CHANNELS`) as well.
pub const INVOKE_CHANNELS: [&str; 3] = ["updater:check", "updater:install", "updater:dismiss"];

/// The first look, once the window and the accounts had their turn.
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// How long a check counts; on the wall clock, which goes on while the computer sleeps.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background task looks whether a check is due. `sleep` alone would not do: its
/// clock stands still while the computer sleeps, and a laptop would seldom get to a day.
const WAKE_EVERY: Duration = Duration::from_secs(60 * 60);
/// How long the first check waits for the window to listen, so that its announcement arrives.
const WINDOW_WAIT: Duration = Duration::from_secs(120);
/// At most this many progress events a second.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// A version that can be installed, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// The release notes of `latest.json` (Markdown: the release's part of `CHANGELOG.md`).
    pub notes: Option<String>,
    /// As `latest.json` has it, RFC 3339.
    pub pub_date: Option<String>,
    /// The version that runs now.
    pub current_version: String,
}

impl UpdateInfo {
    fn of(update: &Update) -> Self {
        UpdateInfo {
            version: update.version.clone(),
            notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
            pub_date: update.raw_json.get("pub_date").and_then(Value::as_str).map(str::to_owned),
            current_version: update.current_version.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Available {
    info: UpdateInfo,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// What the updater keeps during one run of the app (`AppState::updater`).
#[derive(Default)]
pub struct Updates {
    /// The update the last check found, for [`install`].
    pending: Mutex<Option<Update>>,
    seen: Mutex<Seen>,
    /// Between "Install and restart" and the end of this process, or a failed download.
    installing: AtomicBool,
}

/// The versions the window was told about, and the ones it put off with "Later".
#[derive(Debug, Default)]
pub struct Seen {
    announced: HashSet<String>,
    dismissed: HashSet<String>,
}

impl Seen {
    /// Whether a version that was found is announced now: the first time it is found, and not
    /// after "Later". Counts it as announced.
    pub fn announce(&mut self, version: &str) -> bool {
        !self.dismissed.contains(version) && self.announced.insert(version.to_owned())
    }

    /// "Later": the version is not announced again in this run of the app.
    pub fn dismiss(&mut self, version: &str) {
        self.dismissed.insert(version.to_owned());
    }
}

/// Whether the app looks for updates by itself: on unless Settings switched it off.
pub fn automatic(settings: &AppSettings) -> bool {
    settings.check_updates != Some(false)
}

/// Whether a day has passed since the last check (never checked: it has). A clock that was put
/// back counts as due too; the check that follows sets the time again.
fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| now.duration_since(last).map_or(true, |passed| passed >= CHECK_EVERY))
}

/// Starts the background checks: [`FIRST_CHECK`] after the start, then once a day while
/// `checkUpdates` is on. The setting is read before each check, so switching it takes effect
/// without a restart. A development run and an isolated instance (`MULTI_CLAUDE_HOME`) do not
/// look by themselves: an update installed from there would replace the installed app.
pub fn start(app: &AppHandle) {
    let isolated = crate::state::app_root().is_ok_and(|(_, isolated)| isolated);
    if tauri::is_dev() || isolated {
        log::info!("updates: no automatic checks in a development run or an isolated instance");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        wait_for_window(&app).await;
        let mut last: Option<SystemTime> = None;
        loop {
            let enabled = crate::app::state(&app).is_some_and(|state| automatic(&state.settings()));
            if enabled && due(last, SystemTime::now()) {
                last = Some(SystemTime::now());
                match check(&app).await {
                    Ok(Some(info)) => log::info!("updates: {} is available (this is {})", info.version, info.current_version),
                    Ok(None) => log::debug!("updates: this version is the newest"),
                    Err(e) => log::warn!("updates: {}", e.0),
                }
            }
            tokio::time::sleep(WAKE_EVERY).await;
        }
    });
}

/// Waits until the window listens, for at most [`WINDOW_WAIT`].
async fn wait_for_window(app: &AppHandle) {
    let until = Instant::now() + WINDOW_WAIT;
    while Instant::now() < until {
        if crate::app::state(app).is_some_and(|state| is(&state.flags.renderer_ready)) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// The updater, configured as the module comment says.
fn updater(app: &AppHandle) -> CmdResult<Updater> {
    app.updater_builder()
        .restart_after_install(false)
        .installer_arg("/R")
        .build()
        .map_err(|e| CmdError(format!("Updates are not available: {e}")))
}

fn app_state(app: &AppHandle) -> CmdResult<tauri::State<'_, AppState>> {
    crate::app::state(app).ok_or_else(|| CmdError("The app is still starting.".into()))
}

/// Looks for a newer version now. A version found for the first time in this run (and not put off
/// with "Later") is announced to the window as well.
pub async fn check(app: &AppHandle) -> CmdResult<Option<UpdateInfo>> {
    let state = app_state(app)?;
    if state.updater.installing.load(Ordering::SeqCst) {
        return Err("The update is being installed.".into());
    }
    let found = updater(app)?.check().await.map_err(|e| CmdError(format!("Could not check for updates: {e}")))?;
    let Some(update) = found else {
        *lock(&state.updater.pending) = None;
        return Ok(None);
    };
    let info = UpdateInfo::of(&update);
    *lock(&state.updater.pending) = Some(update);
    // a window that does not listen yet would miss it; it is announced at the next check then
    if is(&state.flags.renderer_ready) && lock(&state.updater.seen).announce(&info.version) {
        events::emit(app, UPDATER_AVAILABLE, Available { info: info.clone() });
    }
    Ok(Some(info))
}

/// "Later": the version is not announced again in this run of the app.
pub fn dismiss(state: &AppState, version: &str) {
    lock(&state.updater.seen).dismiss(version);
}

/// "Install and restart": downloads the update the last check found, ends the app as quitting
/// does, installs the update and starts the new version. Returns only when that failed before the
/// app was ended; the window then still works as before.
pub async fn install(app: &AppHandle) -> CmdResult<()> {
    let state = app_state(app)?;
    let update = lock(&state.updater.pending).clone().ok_or_else(|| CmdError("No update is waiting. Check for updates first.".into()))?;
    if state.updater.installing.swap(true, Ordering::SeqCst) {
        return Err("The update is already being installed.".into());
    }
    let bytes = match download(app, &update).await {
        Ok(bytes) => bytes,
        Err(e) => {
            state.updater.installing.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    log::info!("updates: installing {} over {}", update.version, update.current_version);

    // Windows: the installer replaces the program while it runs, so the app ends first, and
    // `install` ends this process as soon as the installer has started
    #[cfg(windows)]
    {
        crate::app::boot::shutdown(&state);
        if let Err(e) = update.install(&bytes) {
            log::error!("updates: the installer could not be started: {e}");
        }
        // only reached when that failed: this version starts again and reopens the saved tabs
        restart(app)
    }

    // macOS: the new bundle takes the place of this one while it runs; when that fails nothing
    // has been ended yet
    #[cfg(not(windows))]
    {
        if let Err(e) = update.install(&bytes) {
            state.updater.installing.store(false, Ordering::SeqCst);
            return Err(CmdError(format!("The update could not be installed: {e}")));
        }
        crate::app::boot::shutdown(&state);
        restart(app)
    }
}

/// Downloads the update and checks its signature, telling the window how far it is.
async fn download(app: &AppHandle, update: &Update) -> CmdResult<Vec<u8>> {
    let mut progress = Progress { downloaded: 0, total: None };
    let mut sent: Option<Instant> = None;
    let result = update
        .download(
            |chunk, total| {
                progress.downloaded += chunk as u64;
                progress.total = total;
                if sent.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) {
                    sent = Some(Instant::now());
                    events::emit(app, UPDATER_PROGRESS, progress);
                }
            },
            || {},
        )
        .await;
    // the last state, which the throttle may have held back
    events::emit(app, UPDATER_PROGRESS, progress);
    // a failed download, or a file whose signature does not match the public key
    result.map_err(|e| CmdError(format!("The update was not installed: {e}")))
}

/// Starts the app anew, without the arguments of this run (see the module comment), and ends this
/// process. Releasing the single-instance lock (`cleanup_before_exit`) before the new process looks
/// for it keeps the new one from handing itself over to this one.
fn restart(app: &AppHandle) -> ! {
    let mut env = app.env();
    env.args_os.truncate(1);
    app.cleanup_before_exit();
    tauri::process::restart(&env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_announced_once() {
        let mut seen = Seen::default();
        assert!(seen.announce("0.6.1"));
        assert!(!seen.announce("0.6.1"));
        // a newer one that appears later in the same run is news again
        assert!(seen.announce("0.6.2"));
    }

    #[test]
    fn later_keeps_a_version_quiet_for_the_run() {
        let mut seen = Seen::default();
        seen.dismiss("0.6.1");
        assert!(!seen.announce("0.6.1"));
        assert!(seen.announce("0.6.2"));
        // put off after it was announced: still not again
        seen.dismiss("0.6.2");
        assert!(!seen.announce("0.6.2"));
    }

    #[test]
    fn checks_are_on_unless_switched_off() {
        let mut settings = AppSettings::default();
        assert!(automatic(&settings));
        settings.check_updates = Some(true);
        assert!(automatic(&settings));
        settings.check_updates = Some(false);
        assert!(!automatic(&settings));
        // as config.json has it
        let settings: AppSettings = serde_json::from_value(serde_json::json!({ "fontSize": 13, "checkUpdates": false })).unwrap();
        assert!(!automatic(&settings));
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(due(None, start));
        assert!(!due(Some(start), start + Duration::from_secs(60)));
        assert!(!due(Some(start), start + CHECK_EVERY - Duration::from_secs(1)));
        assert!(due(Some(start), start + CHECK_EVERY));
        // the clock was put back
        assert!(due(Some(start), start - Duration::from_secs(60)));
    }

    /// What the plugin reads at start: a config it cannot take would end the app there.
    #[test]
    fn the_config_is_one_the_plugin_takes() {
        let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let config: tauri_plugin_updater::Config = serde_json::from_value(conf["plugins"]["updater"].clone()).unwrap();
        assert_eq!(config.endpoints.len(), 1);
        assert_eq!(config.endpoints[0].scheme(), "https");
        assert!(config.endpoints[0].path().ends_with("/releases/latest/download/latest.json"), "{}", config.endpoints[0]);
        assert!(!config.pubkey.trim().is_empty());
        // a latest.json that pairs a new version number with an older release's file is refused
        assert!(config.require_signed_version);
        assert_eq!(config.windows.map(|w| w.install_mode.to_string()).as_deref(), Some("passive"));
        // signed installers and latest.json entries; the macOS archive needs the `app` target
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
        let targets = conf["bundle"]["targets"].as_array().unwrap();
        assert!(["nsis", "app", "dmg"].iter().all(|t| targets.contains(&Value::from(*t))), "{targets:?}");
    }

    #[test]
    fn update_info_is_camel_case() {
        let info = UpdateInfo { version: "0.6.1".into(), notes: None, pub_date: Some("2026-11-20T10:00:00Z".into()), current_version: "0.6.0".into() };
        assert_eq!(
            serde_json::to_value(Available { info }).unwrap(),
            serde_json::json!({ "info": { "version": "0.6.1", "notes": null, "pubDate": "2026-11-20T10:00:00Z", "currentVersion": "0.6.0" } })
        );
        assert_eq!(serde_json::to_value(Progress { downloaded: 10, total: None }).unwrap(), serde_json::json!({ "downloaded": 10, "total": null }));
    }
}
