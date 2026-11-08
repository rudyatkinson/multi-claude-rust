//! Workflows: the designed ones ([`store`]), their runs ([`runs`]) and the opening of a step's
//! session ([`launcher`]), and what `src/main/index.ts` did around them: the checks before a run,
//! what the runs need of the app, the notices and notifications of a run, and the cleanup at
//! startup.

pub mod launcher;
pub mod runs;
pub mod store;

pub use launcher::{start_step, LauncherDeps, END_BEFORE_RESUME_MS};
pub use runs::{Cancelled, RunsDeps, StartedStep, WorkflowRuns};
pub use store::WorkflowStore;

use std::collections::HashSet;
use std::path::PathBuf;

use mc_core::error::{AppError, AppResult};
use mc_core::folders::folder_key;
use mc_core::layout::active_id_of;
use mc_core::transfer::find_transcript;
use mc_core::types::{Place, Profile, SessionInfo, SessionRequest, StartRunInput, WorkflowIssueLevel, WorkflowRun};
use mc_core::workflow::{
    preflight, prune_run_folders, read_last_answer, run_folder, validate_workflow, write_step_output, Notice, RunData, KEEP_RUN_FILES_MS,
};
use serde_json::Value;
use tauri::AppHandle;

use crate::app::launch::{replace_session, start_session, CLAUDE_NOT_FOUND};
use crate::app::{notice, now_ms, remember_folder, state as app_state, window};
use crate::claude_binary::is_shim;
use crate::events::{self, NoticeLevel};
use crate::mcp_servers::BoxFuture;
use crate::sessions::title_of;
use crate::state::AppState;

/// A process that is told to end before its plan dialog can be answered gets this long.
const END_PLAN_MS: u64 = 3000;

/// Sends `workflows:changed` with the designed workflows.
pub fn emit_workflows(state: &AppState) {
    let list = state.workflows.list();
    log::debug!("{}: {} workflows", events::WORKFLOWS_CHANGED, list.len());
    events::emit(&state.app, events::WORKFLOWS_CHANGED, list);
}

/// `saveWorkflow`.
pub fn save_workflow(state: &AppState, raw: &Value) -> AppResult<()> {
    state.workflows.save(raw)?;
    emit_workflows(state);
    Ok(())
}

/// `deleteWorkflow`.
pub fn delete_workflow(state: &AppState, id: &str) -> AppResult<()> {
    state.workflows.remove(id)?;
    emit_workflows(state);
    Ok(())
}

/// `runWorkflow`: begins a run once nothing stands in its way. Its first step is opened after
/// this returns; `place` is where the window wants that step's tab.
pub fn run_workflow(state: &AppState, input: &StartRunInput, place: Option<Place>) -> AppResult<WorkflowRun> {
    let workflow = state.workflows.get(&input.workflow_id).ok_or_else(|| AppError::refused("That workflow no longer exists."))?;
    let claude = state.claude_path().ok_or_else(|| AppError::refused(CLAUDE_NOT_FOUND))?;
    // a step's first message would pass through cmd.exe and its own reading of quotes and "&"
    if is_shim(&claude) {
        return Err(AppError::refused("Workflows need the native claude executable, not the .cmd one. Set its path in Settings."));
    }
    if !state.hooks.is_listening() {
        return Err(AppError::refused(
            "The app could not open the local port that workflows report to. Restart Multi Claude and try again.",
        ));
    }
    let accounts = state.profiles.list();
    let ids: HashSet<String> = accounts.iter().map(|p| p.id.clone()).collect();
    let mut issues = validate_workflow(&workflow, &ids);
    issues.extend(preflight(&workflow, &accounts, &state.usage.snapshot(), now_ms()));
    if let Some(blocking) = issues.into_iter().find(|issue| issue.level == WorkflowIssueLevel::Error) {
        return Err(AppError::refused(blocking.message));
    }
    let run = state.runs.start(&workflow, &input.cwd, &input.task, place)?;
    remember_folder(state, &input.cwd);
    Ok(run)
}

/// At startup: the sample workflow for someone who never designed one, and the folders of runs
/// and skill requests that are long over.
pub fn boot(state: &AppState) {
    if state.workflows.seed(&state.profiles.list()) {
        log::info!("workflows: wrote the sample workflow to {}", state.profiles.paths().workflows_file().display());
        emit_workflows(state);
    }
    let paths = state.profiles.paths();
    let now = now_ms();
    prune_run_folders(&paths.runs_dir(), now, KEEP_RUN_FILES_MS);
    prune_run_folders(&paths.skill_requests_dir(), now, KEEP_RUN_FILES_MS);
}

/// `runNotice`: tells the user that a run needs them, or that it is over. With the window in front
/// a line inside it is enough (and nothing at all when they look at the very tab that asks);
/// otherwise a notification whose click brings the step's tab to the front.
fn run_notice(state: &AppState, run: &WorkflowRun, kind: Notice, index: usize) {
    let Some(step) = run.steps.get(index) else { return };
    let next = run.steps.get(index + 1);
    let (title, body) = match kind {
        Notice::Gate => (
            format!("{} is finished", step.name),
            format!("\"{}\" waits for your approval{}.", run.name, next.map(|n| format!(" before {}", n.name)).unwrap_or_default()),
        ),
        Notice::NeedsYou => (format!("{} needs you", step.name), format!("\"{}\" waits for you in the tab of this step.", run.name)),
        Notice::Paused => (
            format!("{} has stopped", step.name),
            step.problem.clone().unwrap_or_else(|| format!("\"{}\" cannot go on by itself.", run.name)),
        ),
        Notice::Done => {
            let steps = if run.steps.len() == 1 { "step has".to_owned() } else { format!("{} steps have", run.steps.len()) };
            (format!("\"{}\" is finished", run.name), format!("Its {steps} run in {}.", title_of(&run.cwd)))
        }
    };
    if window::is_focused(&state.app) {
        // looking at the very tab that asks: the tab says it all
        if kind == Notice::NeedsYou && step.session_id.is_some() && active_id_of(&state.tabs.layout()) == step.session_id.as_deref() {
            return;
        }
        let level = if kind == Notice::Paused { NoticeLevel::Warn } else { NoticeLevel::Info };
        notice(state, level, format!("{title}. {body}"));
        return;
    }
    if state.settings().notifications == Some(false) {
        return;
    }
    window::flash(&state.app);
    crate::app::notify::show_for(&state.app, &title, &body, step.session_id.as_deref());
}

// ---------- what the runs need of the app ----------

/// [`RunsDeps`] of the app. It finds the state through the handle when it is called.
pub struct AppRuns(pub AppHandle);

impl RunsDeps for AppRuns {
    fn now(&self) -> i64 {
        now_ms()
    }

    fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn folder_key(&self, cwd: &str) -> String {
        folder_key(cwd, cfg!(windows))
    }

    fn start_step(&self, run: RunData, index: usize, cancelled: Cancelled) -> BoxFuture<'static, Result<StartedStep, String>> {
        let launcher = AppLauncher(self.0.clone());
        Box::pin(async move { start_step(&launcher, &run, index, &*cancelled).await.map_err(|e| e.to_string()) })
    }

    fn end_session(&self, session_id: &str) {
        let Some(state) = app_state(&self.0) else { return };
        tauri::async_runtime::spawn(state.sessions.end(session_id, END_PLAN_MS));
    }

    fn last_answer(&self, run: RunData, index: usize) -> BoxFuture<'static, Option<String>> {
        let app = self.0.clone();
        Box::pin(async move {
            let file = {
                let state = app_state(&app)?;
                let step = run.steps.get(index)?;
                let thread = run.threads.get(&step.thread)?;
                let profile = state.profiles.get(thread.profile_id.as_deref().filter(|p| !p.is_empty())?)?;
                let conversation = thread.conversation_id.as_deref().filter(|c| !c.is_empty())?;
                let reported = step.session_id.as_deref().and_then(|tab| state.sessions.transcript_of(tab));
                find_transcript(&state.profiles.paths().config_dir_of(&profile), conversation, reported.as_deref())?
            };
            tauri::async_runtime::spawn_blocking(move || read_last_answer(&file)).await.ok().flatten()
        })
    }

    fn save_output(&self, run: &RunData, index: usize, text: &str) {
        let Some(state) = app_state(&self.0) else { return };
        let Some(step) = run.steps.get(index) else { return };
        let dir = run_folder(&state.profiles.paths().runs_dir(), &run.id);
        let (name, text) = (step.def.name.clone(), text.to_owned());
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = write_step_output(&dir, index, &name, &text) {
                log::debug!("workflow output: {e}");
            }
        });
    }

    fn notify(&self, run: &WorkflowRun, notice: Notice, index: usize) {
        if let Some(state) = app_state(&self.0) {
            run_notice(&state, run, notice, index);
        }
    }

    fn changed(&self, runs: Vec<WorkflowRun>) {
        events::emit(&self.0, events::RUNS_CHANGED, runs);
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(task);
    }
}

/// [`LauncherDeps`] of the app: the accounts, the sessions and the runs folder.
struct AppLauncher(AppHandle);

impl AppLauncher {
    fn with<T>(&self, f: impl FnOnce(&AppState) -> T) -> Option<T> {
        app_state(&self.0).map(|state| f(&state))
    }

    fn with_result<T>(&self, f: impl FnOnce(&AppState) -> AppResult<T>) -> AppResult<T> {
        self.with(f).unwrap_or_else(|| Err(AppError::refused("Multi Claude is shutting down.")))
    }
}

impl LauncherDeps for AppLauncher {
    fn profile(&self, id: &str) -> Option<Profile> {
        self.with(|s| s.profiles.get(id)).flatten()
    }

    fn config_dir(&self, profile: &Profile) -> PathBuf {
        self.with(|s| s.profiles.paths().config_dir_of(profile)).unwrap_or_default()
    }

    fn runs_dir(&self) -> PathBuf {
        self.with(|s| s.profiles.paths().runs_dir()).unwrap_or_default()
    }

    fn first_place(&self, run_id: &str) -> Option<Place> {
        self.with(|s| s.runs.first_place(run_id)).flatten()
    }

    fn create(&self, request: &SessionRequest, place: Option<Place>) -> AppResult<SessionInfo> {
        self.with_result(|s| start_session(s, request, place))
    }

    fn replace(&self, tab: &str, request: &SessionRequest) -> AppResult<SessionInfo> {
        self.with_result(|s| replace_session(s, tab, request))
    }

    fn end(&self, tab: &str, timeout_ms: u64) -> BoxFuture<'static, ()> {
        match app_state(&self.0) {
            Some(state) => Box::pin(state.sessions.end(tab, timeout_ms)),
            None => Box::pin(async {}),
        }
    }

    fn has_session(&self, id: &str) -> bool {
        self.with(|s| s.sessions.get(id).is_some()).unwrap_or(false)
    }

    fn transcript_of(&self, id: &str) -> Option<String> {
        self.with(|s| s.sessions.transcript_of(id)).flatten()
    }
}
