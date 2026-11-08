//! Connects the services to each other, once at startup: what the Electron main process passed to
//! their constructors (`SessionDeps`, `ProfileDeps`, the hook server's callback, the usage
//! tracker's callbacks, the alerts' `turnOf`/`tell`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mc_core::auth_status::parse_auth_status;
use mc_core::hooks::parse_hook;
use mc_core::layout::reconcile;
use mc_core::turn::turn_event_of;
use mc_core::types::{AccountInfo, AuthKind, Profile, SessionInfo, SessionKind, SessionRef};
use mc_core::workflow::RunSession;
use tauri::AppHandle;

use crate::app::{notice, note_turn, now_ms, state as app_state, window};
use crate::claude_cli;
use crate::events::NoticeLevel;
use crate::limit_log;
use crate::mcp_servers::BoxFuture;
use crate::profiles::AccountCli;
use crate::session_alerts::AlertDeps;
use crate::state::AppState;
use crate::usage::{ProbeDeps, ProbeProfile};

/// Sets every handler. Call once, right after the state is managed.
pub fn wire(app: &AppHandle) {
    let Some(state) = app_state(app) else { return };
    wire_sessions(app, &state);
    wire_profiles(app, &state);
    wire_hooks(app, &state);
    wire_usage(app, &state);
    state.alerts.set_deps(Arc::new(Alerts(app.clone())));
    wire_instructions(app, &state);
}

// ---------- instructions ----------

/// "What a session reads" ends with the memory folder the session really uses: the main account's
/// for an account that shares memory, else its own.
fn wire_instructions(app: &AppHandle, state: &AppState) {
    let handle = app.clone();
    state.instructions.set_memory_dir(move |profile_id, cwd| {
        let state = app_state(&handle)?;
        let profile = state.profiles.get(profile_id)?;
        let paths = state.profiles.paths();
        state.memory.session_memory_dir(&paths.main_claude_dir(), &profile, &paths.config_dir_of(&profile), cwd)
    });
}

// ---------- sessions ----------

fn wire_sessions(app: &AppHandle, state: &AppState) {
    // `emitSessions` besides sending: the app's copy of the layout follows the list, so that it
    // never names a session that is gone; the tabs are saved; alerts of gone sessions are dropped.
    let handle = app.clone();
    state.sessions.set_changed_handler(Box::new(move |list: &[SessionInfo]| {
        let state = app_state(&handle)?;
        let refs: Vec<SessionRef> =
            list.iter().map(|s| SessionRef { id: s.id.clone(), replaces: s.replaces.clone(), place: s.place.clone() }).collect();
        state.tabs.update_layout(|layout| reconcile(layout, &refs));
        state.tabs.persist(&state.sessions.saved_tabs());
        state.alerts.prune(list.iter().map(|s| s.id.as_str()));
        state.runs.sessions_changed(list.iter().map(RunSession::from).collect());
        // Phase 4: the tray tooltip counts the sessions
        // a restored arrangement travels with the list that introduces its sessions, and only with it
        state.flags.layout_to_send.swap(false, std::sync::atomic::Ordering::SeqCst).then(|| state.tabs.layout())
    }));

    // `onExit`
    let handle = app.clone();
    state.sessions.set_exit_handler(Box::new(move |info: SessionInfo| {
        let Some(state) = app_state(&handle) else { return };
        if info.kind != SessionKind::Login {
            // the session's last requests are not in the status line data yet
            state.usage.request_refresh(&info.profile_id);
            return;
        }
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            let Some(state) = app_state(&handle) else { return };
            let warning = state.profiles.refresh_status(&info.profile_id).await;
            let Some(profile) = state.profiles.get(&info.profile_id) else { return };
            state.usage.request_refresh(&info.profile_id);
            let account = profile.account.as_ref();
            if let Some(warning) = warning {
                notice(&state, NoticeLevel::Warn, warning);
            } else if account.is_some_and(|a| a.logged_in) {
                let email = account.and_then(|a| a.email.as_deref()).map(|e| format!(" as {e}")).unwrap_or_default();
                notice(&state, NoticeLevel::Info, format!("\"{}\" is signed in{email}.", profile.name));
            } else {
                notice(&state, NoticeLevel::Warn, format!("The login for \"{}\" was not completed.", profile.name));
            }
        });
    }));

    // `cleanupSettings`
    let handle = app.clone();
    state.sessions.set_cleanup_handler(Box::new(move |profile_id: &str, session_id: &str| {
        let Some(state) = app_state(&handle) else { return };
        state.usage.session_ended(profile_id, session_id);
        state.hooks.release(session_id);
    }));
}

// ---------- profiles ----------

/// `claude auth ...` with the account's environment and the claude that was found.
struct Cli(AppHandle);

impl Cli {
    /// The claude binary and the account's environment, read when the call is made.
    fn prepare(&self, profile: &Profile) -> Option<(String, BTreeMap<String, String>)> {
        let state = app_state(&self.0)?;
        Some((state.claude_path()?, state.profiles.env_for(profile)))
    }
}

impl AccountCli for Cli {
    fn available(&self) -> bool {
        app_state(&self.0).is_some_and(|s| s.claude_path().is_some())
    }

    fn status(&self, profile: Profile) -> BoxFuture<'static, AccountInfo> {
        let prepared = self.prepare(&profile);
        Box::pin(async move {
            match prepared {
                Some((claude, env)) => claude_cli::auth_status(&claude, &env).await,
                None => AccountInfo { error: Some("claude was not found".into()), ..parse_auth_status("", now_ms()) },
            }
        })
    }

    fn logout(&self, profile: Profile) -> BoxFuture<'static, ()> {
        let prepared = self.prepare(&profile);
        Box::pin(async move {
            if let Some((claude, env)) = prepared {
                claude_cli::auth_logout(&claude, &env).await;
            }
        })
    }
}

fn wire_profiles(app: &AppHandle, state: &AppState) {
    state.profiles.set_cli(Arc::new(Cli(app.clone())));
    let sessions = state.sessions.clone();
    state.profiles.set_close_sessions(move |profile_id| sessions.close_for_profile(profile_id));
    // `changed`: after `profiles:changed`, the launchers follow the account list
    let handle = app.clone();
    state.profiles.set_on_changed(move || {
        if let Some(state) = app_state(&handle) {
            state.profiles.refresh_launchers(state.claude_path().as_deref());
        }
    });
}

// ---------- hooks ----------

fn wire_hooks(app: &AppHandle, state: &AppState) {
    // `onHook`: notes failed turns and limit waits, then moves the session's turn along
    let handle = app.clone();
    state.hooks.set_handler(Box::new(move |session_id, event, body| {
        let Some(state) = app_state(&handle) else { return };
        let info = state.sessions.get(&session_id);
        let account = info.as_ref().and_then(|i| state.profiles.get(&i.profile_id)).map(|p| p.name).unwrap_or_default();
        limit_log::record_hook(&state.root, event, &body, &account);
        let Some(report) = parse_hook(event, &body) else { return };
        state.runs.hook(&session_id, &report);
        note_turn(&state, &session_id, &turn_event_of(&report.signal));
    }));
}

// ---------- usage ----------

/// The accounts, the claude binary and the environment the usage probe runs with.
struct Probe(AppHandle);

impl ProbeDeps for Probe {
    fn claude_path(&self) -> Option<PathBuf> {
        app_state(&self.0)?.claude_path().map(PathBuf::from)
    }

    fn profiles(&self) -> Vec<ProbeProfile> {
        let Some(state) = app_state(&self.0) else { return Vec::new() };
        state
            .profiles
            .list()
            .into_iter()
            .map(|p| ProbeProfile {
                logged_in: p.account.as_ref().is_some_and(|a| a.logged_in),
                token: p.auth == Some(AuthKind::Token),
                id: p.id,
                name: p.name,
                config_dir: p.config_dir,
            })
            .collect()
    }

    fn env_for(&self, profile: &ProbeProfile) -> BTreeMap<String, String> {
        let found = app_state(&self.0).and_then(|s| s.profiles.get(&profile.id).map(|p| s.profiles.env_for(&p)));
        found.unwrap_or_else(|| claude_cli::profile_env(profile.config_dir.as_deref().map(Path::new), None))
    }
}

fn wire_usage(app: &AppHandle, state: &AppState) {
    state.usage.set_deps(Arc::new(Probe(app.clone())));
    // `deliverAlerts`: a line in the window when it is in front, else a system notification
    let handle = app.clone();
    state.usage.set_notifier(move |_profile_id, message| {
        let Some(state) = app_state(&handle) else { return };
        if state.profiles.config().with(|c| c.settings.limit_alerts) == Some(false) {
            return;
        }
        if window::is_focused(&handle) {
            let text = if message.body.is_empty() { message.title } else { format!("{}. {}", message.title, message.body) };
            notice(&state, message.level, text);
        } else {
            crate::app::notify::show(&handle, &message.title, &message.body);
        }
    });
    let sessions = state.sessions.clone();
    state.usage.set_session_meta_sink(move |session_id, meta| sessions.set_meta(session_id, &meta));
}

// ---------- alerts ----------

/// What the session alerts read of the rest of the app.
struct Alerts(AppHandle);

impl AlertDeps for Alerts {
    fn session(&self, id: &str) -> Option<SessionInfo> {
        app_state(&self.0)?.sessions.get(id)
    }

    fn profile_name(&self, profile_id: &str) -> Option<String> {
        app_state(&self.0)?.profiles.get(profile_id).map(|p| p.name)
    }

    fn notifications_on(&self) -> bool {
        app_state(&self.0).is_some_and(|s| s.profiles.config().with(|c| c.settings.notifications) != Some(false))
    }

    fn is_workflow_step(&self, id: &str) -> bool {
        app_state(&self.0).is_some_and(|s| s.runs.watches(id))
    }
}
