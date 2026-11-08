//! Starting and ending the app (the bottom of `src/main/index.ts`): the services are made and
//! connected, the tray icon and the start with the system are set up, claude is looked for, the
//! accounts are checked, the usage probe starts; a second instance hands over its arguments;
//! quitting saves the tabs and ends the sessions.

use std::sync::atomic::Ordering;

use mc_core::open_folder_args::{parse_startup_args, StartupArgs};
use sha1::{Digest, Sha1};
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{App, AppHandle, Manager, RunEvent, Webview};

use crate::app::open_folder::{flush_pending, request_open_folder};
use crate::app::window::{self, bring_to_front, first_show};
use crate::app::{locate_claude, refresh_all_profiles, state as app_state};
use crate::state::{is, lock, AppState};

/// What this process was started for, read from its own command line.
pub fn startup_args() -> StartupArgs {
    let argv: Vec<String> = std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
    parse_startup_args(&argv)
}

/// An isolated instance (`MULTI_CLAUDE_HOME`) keeps its own webview data and its own instance
/// lock, both of which Tauri derives from the identifier, as the Electron version moved its
/// `userData` into the isolated folder.
pub fn isolate(context: &mut tauri::Context, root: &std::path::Path) {
    let digest = Sha1::digest(root.to_string_lossy().as_bytes());
    let config = context.config_mut();
    config.identifier = format!("{}.isolated-{}", config.identifier, &hex::encode(digest)[..8]);
}

/// The setup hook: makes the state, connects the services, starts what runs in the background
/// and the boot sequence.
pub fn setup(app: &mut App, root: std::path::PathBuf, startup: StartupArgs) -> Result<(), Box<dyn std::error::Error>> {
    if startup.quit {
        // asked to end the running instance, and there is none
        log::info!("--quit: no running instance");
        app.cleanup_before_exit();
        std::process::exit(0);
    }
    let handle = app.handle().clone();
    let state = AppState::new(handle.clone(), root, startup)?;
    if let Err(e) = state.profiles.paths().ensure_dirs() {
        log::warn!("creating the app's folders under {}: {e}", state.root.display());
    }
    app.manage(state);
    crate::app::wiring::wire(&handle);
    let state = handle.state::<AppState>();
    log::debug!("state built: {} ({} accounts)", state.root.display(), state.profiles.list().len());

    state.usage.prune(state.profiles.list().iter().map(|p| p.id.as_str()));
    state.usage.start();
    crate::sleep::start(&handle);
    crate::workflows::boot(&state);
    crate::updater::start(&handle);
    // without it the app works as before: tabs do not report their turns, and workflows cannot run
    if let Err(e) = state.hooks.start() {
        log::warn!("{e}");
        *lock(&state.hook_problem) = Some(e.to_string());
    }
    shell_integration(&handle, &state);

    // a window whose page never reports ready is shown anyway
    let fallback = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(window::SHOW_FALLBACK).await;
        if let Some(state) = app_state(&fallback) {
            if !is(&state.flags.window_shown) && !is(&state.flags.start_hidden) {
                log::warn!("the window did not report ready; showing it anyway");
                window::show(&state);
            }
        }
    });
    tauri::async_runtime::spawn(boot(handle));
    Ok(())
}

/// The tray icon, the macOS app menu and the start with the system. Whether a tray icon was made
/// decides what `--hidden` and closing the window do.
fn shell_integration(app: &AppHandle, state: &AppState) {
    // once for the app: the tray menu is built again and again
    app.on_menu_event(|app, event| {
        crate::tray::on_menu_event(app, &event);
        #[cfg(target_os = "macos")]
        {
            crate::app::quit::on_menu_event(app, &event);
        }
    });
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = crate::app::quit::install_app_menu(app) {
            log::warn!("app menu: {e}");
        }
    }
    let tray = state.settings().tray != Some(false) && crate::tray::create(app);
    // without a tray icon a hidden window could not be brought back
    state.flags.start_hidden.store(state.startup.hidden && tray, Ordering::SeqCst);
    // the uninstaller removes the entry, and an update may have moved the app
    if state.settings().open_at_login.is_some() {
        crate::app::login_item::apply(state, false);
    }
}

/// The part of the start that waits for processes: claude is looked for (on macOS with the login
/// shell's PATH), then folders and restored tabs may go to the window, then the accounts are
/// checked and the usage probe starts.
async fn boot(app: AppHandle) {
    let Some(state) = app_state(&app) else { return };
    crate::claude_binary::import_login_shell_path().await;
    locate_claude(&state).await;
    state.flags.boot_ready.store(true, Ordering::SeqCst);
    if let Some(folder) = state.startup.open_folder.clone() {
        request_open_folder(&state, &folder);
    }
    flush_pending(&state);
    refresh_all_profiles(&state).await;
    if log::log_enabled!(log::Level::Debug) {
        log::debug!("accounts checked: {}", account_summary(&state));
    }
    state.usage.start_prober();
    // after an update or a moved install the menu entry has to point at this executable again
    if state.settings().explorer_menu == Some(true) {
        if let Err(e) = state.explorer_menu.ensure().await {
            log::warn!("explorer menu: {e}");
        }
    }
}

fn account_summary(state: &AppState) -> String {
    let list: Vec<String> = state
        .profiles
        .list()
        .iter()
        .map(|p| match &p.account {
            Some(a) if a.logged_in => format!("{} signed in", p.name),
            Some(a) => format!("{} signed out{}", p.name, a.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default()),
            None => format!("{} not checked", p.name),
        })
        .collect();
    list.join(", ")
}

/// `app:ready`: the renderer listens. The window appears the first time; the hook server's
/// failure is told once; what waited for the window goes out.
pub fn renderer_ready(state: &AppState) {
    state.flags.renderer_ready.store(true, Ordering::SeqCst);
    if let Some(problem) = lock(&state.hook_problem).take() {
        crate::app::notice(
            state,
            crate::events::NoticeLevel::Warn,
            format!("{problem}. Tabs cannot show when Claude is done or waits for you."),
        );
    }
    first_show(state);
    flush_pending(state);
    crate::updater::tell_again(state);
}

/// A page started loading in the main window (a reload): until it reports ready again nothing is
/// delivered to it, and what it had typed is gone with it.
pub fn on_page_load(webview: &Webview, payload: &PageLoadPayload<'_>) {
    if webview.label() != window::MAIN || payload.event() != PageLoadEvent::Started {
        return;
    }
    if let Some(state) = app_state(webview.app_handle()) {
        state.flags.renderer_ready.store(false, Ordering::SeqCst);
        lock(&state.unsaved).clear();
    }
}

/// The single-instance callback: another start of the app handed over its command line.
pub fn second_instance(app: &AppHandle, argv: Vec<String>) {
    let request = parse_startup_args(&argv);
    let Some(state) = app_state(app) else { return };
    if request.quit {
        // a script or an installer asks; nobody is there to answer a question
        shutdown(&state);
        app.exit(0);
        return;
    }
    bring_to_front(&state);
    if let Some(folder) = request.open_folder {
        request_open_folder(&state, &folder);
    }
}

/// Every way of quitting ends here (`shutdown`): the tabs are saved while the sessions are still
/// alive, then the background work stops and the sessions end. Safe to call more than once.
pub fn shutdown(state: &AppState) {
    if state.flags.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    state.tabs.persist(&state.sessions.saved_tabs());
    state.tabs.quit();
    crate::tray::destroy(&state.app);
    state.runs.stop_all();
    state.auto_switch.stop();
    state.usage.stop();
    state.sessions.kill_all();
    state.hooks.stop();
}

/// The run loop's events. `Exit` is also how Windows ending the session (logoff, shutdown, an
/// installer's restart manager) arrives: tao turns `WM_ENDSESSION` into the normal exit path (it
/// ends the event loop, which Tauri reports as `RunEvent::Exit`, and then exits the process
/// itself), so [`shutdown`] saves the tabs there, synchronously, as the `session-end` handler of
/// the Electron version did. Quitting from the window, the tray or Cmd+Q asks first and arrives
/// here already shut down (`quit::request_quit`).
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            if let Some(state) = app_state(app) {
                shutdown(&state);
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { has_visible_windows, .. } => {
            // a click on the Dock icon brings back the window
            if !has_visible_windows {
                if let Some(state) = app_state(app) {
                    bring_to_front(&state);
                }
            }
        }
        _ => {}
    }
}
