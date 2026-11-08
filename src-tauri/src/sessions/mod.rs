//! Pty sessions: spawn, write, resize, kill, replay buffer, `pty:data` and `sessions:changed`
//! events (`src/main/sessions.ts`).
//!
//! A session is a program in a pseudo terminal. Each one has four threads: one writes its input,
//! one reads its output into a [`stream::Stream`], one sends that output to the window in chunks,
//! and one waits for the program to end. The list of sessions is kept in one lock; nothing that
//! can block (closing a Windows pseudoconsole, the handlers below) runs while it is held.
//!
//! What the Electron version took as `SessionDeps` comes in two parts here: the profile, the
//! claude path and the environment go into a [`SpawnPlan`] (see [`SessionManager::plan`]), and
//! the reactions are handlers set once at startup: [`SessionManager::set_changed_handler`] (the
//! work `emitSessions` did besides sending), [`SessionManager::set_exit_handler`] (`onExit`) and
//! [`SessionManager::set_cleanup_handler`] (`cleanupSettings`).
//!
//! A Claude session can also sleep ([`SessionManager::sleep`]): its process ends, but the session
//! stays in the list under its id, as [`SessionStatus::Sleeping`], with its output, name and
//! conversation. [`SessionManager::wake`] starts a new process under the same id; its output
//! continues the old replay and `seq`, so the window keeps the tab and its terminal.

mod output;
mod pty;
mod stream;

pub use output::{FLUSH_MS, FLUSH_UNITS, REPLAY_LIMIT};
pub use pty::{COLS, ROWS};

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mc_core::claude_args::claude_args_with;
use mc_core::error::{AppError, AppResult};
use mc_core::terminal_input::is_typing;
use mc_core::types::{
    Arrival, AttachResult, HookNeed, LaunchOptions, Layout, Place, PtyData, SavedTab, SessionInfo, SessionKind, SessionRequest,
    SessionStatus, SessionsChanged, Turn,
};
use mc_core::validate::{PTY_MAX, PTY_MIN};
use portable_pty::{Child, ChildKiller, PtySize};
use tauri::AppHandle;
use tokio::sync::watch;

use crate::claude_binary::{is_shim, spawn_spec};
use pty::Console;
use stream::Stream;

/// How long an ended program gets to hand over the rest of its output.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(1000);
/// Windows: how long [`SessionManager::end`] lets a program end by itself once its console is
/// closed, before it is killed.
const END_GRACE: Duration = Duration::from_millis(1500);

/// Where a session manager sends its events. The app sends them to the window
/// ([`SessionManager::new`]); tests record them.
pub trait Outlet: Send + Sync + 'static {
    /// `pty:data`.
    fn data(&self, payload: PtyData);
    /// `sessions:changed`.
    fn sessions(&self, payload: SessionsChanged);
}

struct WindowOutlet(AppHandle);

impl Outlet for WindowOutlet {
    fn data(&self, payload: PtyData) {
        crate::events::emit(&self.0, crate::events::PTY_DATA, payload);
    }
    fn sessions(&self, payload: SessionsChanged) {
        crate::events::emit(&self.0, crate::events::SESSIONS_CHANGED, payload);
    }
}

/// What the status line reports about a session: its conversation, model and context use. The
/// usage tracker reads it from the status files and hands it to [`SessionManager::set_meta`].
pub use mc_core::usage::SessionMeta;

/// Everything a session is started from, decided before anything starts: the account's
/// environment, the program and its arguments. [`SessionManager::plan`] builds one from a
/// request the way `spawn` of the Electron version did; restarts, moves, workflow steps and
/// restored tabs adjust `place`, `name` or `request` before handing it to
/// [`SessionManager::create`], [`SessionManager::start`] or [`SessionManager::replace`].
#[derive(Clone)]
pub struct SpawnPlan {
    /// The new session's id (a UUID). The settings file and the hook address were made for it.
    pub id: String,
    pub profile_id: String,
    pub kind: SessionKind,
    pub cwd: String,
    /// The program to start: claude, or cmd.exe for a shim (see `claude_binary::spawn_spec`).
    pub program: String,
    pub args: Vec<String>,
    /// The whole environment of the process (it may hold the account's token).
    pub env: BTreeMap<String, String>,
    pub title: String,
    /// The user's name for the tab; only a Claude tab has one.
    pub name: Option<String>,
    /// Where the window should put the tab; sent in the one list that introduces the session.
    pub place: Option<Place>,
    /// The session whose tab this one takes over; set by [`SessionManager::replace`].
    pub replaces: Option<String>,
    /// Claude Code's id of the conversation, when it is known up front.
    pub claude_session_id: Option<String>,
    /// What the session was asked to be, kept for a restart. Its `prompt` is not kept.
    pub request: SessionRequest,
}

impl std::fmt::Debug for SpawnPlan {
    /// Leaves the environment out: it may hold a token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnPlan")
            .field("id", &self.id)
            .field("profile_id", &self.profile_id)
            .field("kind", &self.kind)
            .field("cwd", &self.cwd)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env", &format_args!("{} variables", self.env.len()))
            .field("title", &self.title)
            .field("name", &self.name)
            .field("place", &self.place)
            .field("replaces", &self.replaces)
            .field("claude_session_id", &self.claude_session_id)
            .finish_non_exhaustive()
    }
}

type ExitFn = dyn Fn(SessionInfo) + Send + Sync;
type ChangedFn = dyn Fn(&[SessionInfo]) -> Option<Layout> + Send + Sync;
type CleanupFn = dyn Fn(&str, &str) + Send + Sync;

/// `onExit`: a session's process ended (called once its list was sent).
pub type ExitHandler = Box<ExitFn>;
/// Called with every list before it is sent, as `emitSessions` was: the layout to send with it,
/// if any. It may call back into the manager (an announcement it causes is sent right after),
/// but it must not wait for a lock that a caller of the manager holds.
pub type ChangedHandler = Box<ChangedFn>;
/// `cleanupSettings(profileId, sessionId)`: the session's settings file and hook address can go.
pub type CleanupHandler = Box<CleanupFn>;

#[derive(Default)]
struct Handlers {
    exit: Option<Arc<ExitFn>>,
    changed: Option<Arc<ChangedFn>>,
    cleanup: Option<Arc<CleanupFn>>,
}

/// The pty of a running session.
struct Pty {
    /// Taken when the pty is closed early (an end or a kill on Windows).
    master: Option<Box<dyn Console>>,
    input: pty::Input,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

struct Live {
    info: SessionInfo,
    /// What the session was asked to be, without its first message.
    request: SessionRequest,
    /// Where Claude Code keeps the conversation, as reported by its status line.
    transcript_path: Option<String>,
    /// None once the process has exited.
    pty: Option<Pty>,
    /// The process has been told to end.
    ending: bool,
    stream: Arc<Stream>,
    exited: watch::Receiver<bool>,
    /// `place` and `replaces`, for the one list that introduces the session.
    intro: Option<(Option<Place>, Option<String>)>,
    /// When something was last typed or pasted into the tab (ms since the epoch); its start before that.
    last_input_at: i64,
    /// The size the window last gave the terminal (columns, rows), for a process started in it later.
    size: Option<(u16, u16)>,
    /// The process was told to end so that the session sleeps: its end makes it
    /// [`SessionStatus::Sleeping`] instead of [`SessionStatus::Exited`].
    to_sleep: bool,
}

/// A process started in a pty for a session, with its threads, before its waiter runs.
struct Launched {
    pty: Pty,
    child: Box<dyn Child + Send + Sync>,
    stream: Arc<Stream>,
    exited_tx: watch::Sender<bool>,
    exited_rx: watch::Receiver<bool>,
}

/// A tab that comes back asleep from the last run ([`SessionManager::create_sleeping`]): nothing
/// is started for it until it is woken.
#[derive(Debug, Clone)]
pub struct SleepingTab {
    /// The new session's id.
    pub id: String,
    pub profile_id: String,
    pub cwd: String,
    pub name: Option<String>,
    pub claude_session_id: Option<String>,
    pub transcript_path: Option<String>,
    /// What it is started as when it wakes; its `resume` is decided then.
    pub request: SessionRequest,
}

/// What the idle check reads of a session (see `crate::sleep`).
#[derive(Debug, Clone)]
pub struct Activity {
    pub info: SessionInfo,
    /// When something was last typed or pasted into it; its start before that.
    pub last_input_at: i64,
    /// When it last printed something; its start before that.
    pub last_output_at: i64,
}

impl Live {
    /// Tells the process to end, once. On Windows the pseudoconsole is closed as well, which ends
    /// whatever else runs in it (node-pty did both); the caller closes the returned console.
    fn kill(&mut self) -> Option<Box<dyn Console>> {
        if self.ending {
            return None;
        }
        let pty = self.pty.as_mut()?;
        self.ending = true;
        if let Err(e) = pty.killer.kill() {
            // already gone
            log::debug!("kill {}: {e}", self.info.id);
        }
        if cfg!(windows) {
            pty.master.take()
        } else {
            None
        }
    }

    /// The first step of [`SessionManager::end`]. On Windows only the pseudoconsole is closed
    /// (the caller closes the returned console): the programs in it are asked to close, as when a
    /// console window is closed (`CTRL_CLOSE_EVENT`), and claude writes the end of its
    /// conversation before it goes. Elsewhere the process is told to end at once.
    fn end_gently(&mut self) -> Option<Box<dyn Console>> {
        if !cfg!(windows) {
            return self.kill();
        }
        if self.ending {
            return None;
        }
        self.pty.as_mut()?.master.take()
    }
}

#[derive(Default)]
struct Table {
    live: HashMap<String, Live>,
    order: Vec<String>,
}

impl Table {
    fn remove(&mut self, id: &str) -> Option<Live> {
        let live = self.live.remove(id)?;
        self.order.retain(|x| x != id);
        Some(live)
    }
}

/// An announcement asked for while one was being made on the same thread.
#[derive(Default)]
struct Again {
    pending: bool,
    layout: Option<Layout>,
}

thread_local! {
    /// This thread is inside [`SessionManager::announce`] (in the changed handler).
    static ANNOUNCING: Cell<bool> = const { Cell::new(false) };
}

struct Inner {
    outlet: Arc<dyn Outlet>,
    root: PathBuf,
    table: Mutex<Table>,
    handlers: RwLock<Handlers>,
    /// One announcement at a time, so that lists go out in the order they were taken.
    announcing: Mutex<()>,
    again: Mutex<Again>,
}

/// The sessions of the app. Cloning gives another handle to the same sessions.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}

/// A new session id.
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What a session's threads are named after: the start of its id.
fn tag_of(id: &str) -> String {
    id.chars().take(8).collect()
}

/// `fs.statSync(cwd).isDirectory()`, worded as the Electron version worded it.
pub fn check_folder(cwd: &str) -> AppResult<()> {
    if std::fs::metadata(cwd).is_ok_and(|m| m.is_dir()) {
        Ok(())
    } else {
        Err(AppError::refused(format!("Folder not found: {cwd}")))
    }
}

/// `path.basename` of Node for this platform.
fn basename(p: &str) -> &str {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = p.trim_end_matches(is_sep);
    let start = trimmed.rfind(is_sep).map_or(0, |i| i + 1);
    let name = &trimmed[start..];
    let b = name.as_bytes();
    if cfg!(windows) && start == 0 && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        // `C:` is a drive and `C:foo` names foo on it
        return &name[2..];
    }
    name
}

/// The automatic title of a Claude tab: the folder's name, or the whole path when it has none.
pub fn title_of(cwd: &str) -> String {
    match basename(cwd) {
        "" => cwd.to_owned(),
        name => name.to_owned(),
    }
}

/// Closes a pty on a thread of its own: closing a Windows pseudoconsole can take a moment.
fn close_later(master: Option<Box<dyn Console>>) {
    if let Some(master) = master {
        // if no thread can be started, the console is closed here
        let _ = pty::thread("pty-close".into(), move || drop(master));
    }
}

/// Waits at most `limit` for a session's process to be gone; true when it is.
async fn exited_within(exited: &mut watch::Receiver<bool>, limit: Duration) -> bool {
    let gone = async {
        while !*exited.borrow_and_update() {
            // the waiter is gone with its thread
            if exited.changed().await.is_err() {
                break;
            }
        }
    };
    tokio::time::timeout(limit, gone).await.is_ok()
}

impl SessionManager {
    pub fn new(app: AppHandle, root: PathBuf) -> Self {
        Self::with_outlet(Arc::new(WindowOutlet(app)), root)
    }

    /// A manager that sends its events somewhere else than the window.
    pub fn with_outlet(outlet: Arc<dyn Outlet>, root: PathBuf) -> Self {
        SessionManager {
            inner: Arc::new(Inner {
                outlet,
                root,
                table: Mutex::new(Table::default()),
                handlers: RwLock::new(Handlers::default()),
                announcing: Mutex::new(()),
                again: Mutex::new(Again::default()),
            }),
        }
    }

    /// `~/.multi-claude` (or `MULTI_CLAUDE_HOME`).
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn set_exit_handler(&self, handler: ExitHandler) {
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).exit = Some(Arc::from(handler));
    }

    pub fn set_changed_handler(&self, handler: ChangedHandler) {
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).changed = Some(Arc::from(handler));
    }

    pub fn set_cleanup_handler(&self, handler: CleanupHandler) {
        self.inner.handlers.write().unwrap_or_else(|e| e.into_inner()).cleanup = Some(Arc::from(handler));
    }

    fn handlers(&self) -> std::sync::RwLockReadGuard<'_, Handlers> {
        self.inner.handlers.read().unwrap_or_else(|e| e.into_inner())
    }

    fn cleanup(&self, profile_id: &str, session_id: &str) {
        let handler = self.handlers().cleanup.clone();
        if let Some(cleanup) = handler {
            cleanup(profile_id, session_id);
        }
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        lock(&self.inner.table)
    }

    // ---------- starting ----------

    /// The part of `spawn` that decides how a session starts. The caller has found the profile
    /// (`Profile not found`) and claude (`claude was not found. Set its path in Settings.`) and
    /// built the account's environment. A Claude session's settings file is written by
    /// `prepare_settings(session_id, hooks)` (the Electron `prepareSettings`); when that or the
    /// arguments fail, the cleanup handler is called for the new id.
    pub fn plan(
        &self,
        request: &SessionRequest,
        claude: &str,
        email_hint: Option<&str>,
        env: BTreeMap<String, String>,
        prepare_settings: impl FnOnce(&str, HookNeed) -> AppResult<Option<PathBuf>>,
    ) -> AppResult<SpawnPlan> {
        self.plan_as(new_session_id(), request, claude, email_hint, env, prepare_settings)
    }

    /// [`Self::plan`] for a session of a given id: one that wakes from its sleep keeps its own.
    pub fn plan_as(
        &self,
        id: String,
        request: &SessionRequest,
        claude: &str,
        email_hint: Option<&str>,
        env: BTreeMap<String, String>,
        prepare_settings: impl FnOnce(&str, HookNeed) -> AppResult<Option<PathBuf>>,
    ) -> AppResult<SpawnPlan> {
        check_folder(&request.cwd)?;
        let (args, claude_session_id) = match request.kind {
            SessionKind::Login => {
                let mut args: Vec<String> = ["auth", "login", "--claudeai"].map(str::to_owned).to_vec();
                if let Some(hint) = email_hint.filter(|h| !h.is_empty()) {
                    args.push("--email".into());
                    args.push(hint.to_owned());
                }
                (args, None)
            }
            SessionKind::Claude => {
                // cmd.exe would read parts of a first message as commands
                if request.prompt.as_deref().is_some_and(|p| !p.is_empty()) && is_shim(claude) {
                    return Err(AppError::refused(
                        "A session that starts with a message needs the native claude executable, not the .cmd one.",
                    ));
                }
                let hooks = if request.launch.as_ref().and_then(|l| l.watch) == Some(true) { HookNeed::Required } else { HookNeed::Wanted };
                let built = prepare_settings(&id, hooks).and_then(|settings| {
                    claude_args_with(request.resume.as_ref(), request.launch.as_ref(), request.prompt.as_deref(), settings.as_deref(), new_session_id)
                });
                match built {
                    Ok(built) => (built.args, built.claude_session_id),
                    Err(e) => {
                        self.cleanup(&request.profile_id, &id);
                        return Err(e);
                    }
                }
            }
        };
        let spec = spawn_spec(claude, &args);
        let (title, name) = match request.kind {
            SessionKind::Login => ("Login".to_owned(), None),
            SessionKind::Claude => (title_of(&request.cwd), request.name.clone()),
        };
        Ok(SpawnPlan {
            id,
            profile_id: request.profile_id.clone(),
            kind: request.kind,
            cwd: request.cwd.clone(),
            program: spec.program,
            args: spec.args,
            env,
            title,
            name,
            place: None,
            replaces: None,
            claude_session_id,
            request: SessionRequest { prompt: None, ..request.clone() },
        })
    }

    /// Starts a session and announces it. Its `place` travels with the list that introduces it.
    pub fn create(&self, plan: SpawnPlan) -> AppResult<SessionInfo> {
        let info = self.start(plan)?;
        self.emit_changed(None);
        Ok(info)
    }

    /// Starts the process and adds the session to the list without announcing it.
    pub fn start(&self, plan: SpawnPlan) -> AppResult<SessionInfo> {
        self.spawn(plan, None).map(|(info, _)| info)
    }

    /// `createMany`: starts several sessions without announcing them one by one; [`Self::announce`]
    /// does that for all at once. A session that cannot start is None in the result.
    pub fn create_many(&self, plans: Vec<SpawnPlan>) -> Vec<Option<SessionInfo>> {
        plans
            .into_iter()
            .map(|plan| match self.start(plan) {
                Ok(info) => Some(info),
                Err(e) => {
                    log::warn!("a session could not start: {e}");
                    None
                }
            })
            .collect()
    }

    /// Puts a new session into the tab of an existing one. The old session only goes once the new
    /// process runs, so a failed start leaves the tab as it was. The list is announced once, in
    /// which the new session names the one it replaces.
    pub fn replace(&self, old_id: &str, mut plan: SpawnPlan) -> AppResult<SessionInfo> {
        if !self.table().live.contains_key(old_id) {
            // the plan's settings file and hook address were made for a session that never starts
            self.cleanup(&plan.profile_id, &plan.id);
            return Err(AppError::refused("Session not found"));
        }
        plan.replaces = Some(old_id.to_owned());
        let (info, old) = self.spawn(plan, Some(old_id))?;
        if let Some(old) = old {
            self.dispose(old);
        }
        self.emit_changed(None);
        Ok(info)
    }

    /// Starts the process and puts the session into the list: at the place of `replacing`, which
    /// leaves the list in the same step and is returned, or at the end.
    fn spawn(&self, plan: SpawnPlan, replacing: Option<&str>) -> AppResult<(SessionInfo, Option<Live>)> {
        let (profile_id, id) = (plan.profile_id.clone(), plan.id.clone());
        let result = self.spawn_inner(plan, replacing);
        if result.is_err() {
            self.cleanup(&profile_id, &id);
        }
        result
    }

    /// Starts `program` in a pty for the session `id`, with its writer, reader and flusher
    /// threads. Its output follows the one of `before` when given (a session woken from its sleep).
    #[allow(clippy::too_many_arguments)]
    fn launch(
        &self,
        id: &str,
        program: &str,
        args: &[String],
        cwd: &str,
        env: &BTreeMap<String, String>,
        size: Option<(u16, u16)>,
        before: Option<&Stream>,
    ) -> AppResult<Launched> {
        let opened = pty::open(program, args, cwd, env, size).map_err(AppError::io)?;
        let outlet = self.inner.outlet.clone();
        let stream = match before {
            Some(before) => Stream::following(id.to_owned(), outlet, before),
            None => Stream::new(id.to_owned(), outlet),
        };
        let started = pty::start(opened, stream.clone(), &tag_of(id)).map_err(AppError::io)?;
        let pty::Started { master, child, killer, input } = started;
        let (exited_tx, exited_rx) = watch::channel(false);
        Ok(Launched { pty: Pty { master: Some(Box::new(master)), input, killer }, child, stream, exited_tx, exited_rx })
    }

    /// The thread that waits for a launched process to end. It goes last: the session must be in
    /// the list when its process ends.
    fn start_waiter(
        &self,
        id: &str,
        profile_id: &str,
        stream: Arc<Stream>,
        child: Box<dyn Child + Send + Sync>,
        exited: watch::Sender<bool>,
    ) -> std::io::Result<()> {
        let manager = self.clone();
        let (wait_id, wait_profile) = (id.to_owned(), profile_id.to_owned());
        pty::thread(format!("pty-wait-{}", tag_of(id)), move || {
            manager.wait_exit(&wait_id, &wait_profile, &stream, child, exited);
        })
    }

    fn spawn_inner(&self, plan: SpawnPlan, replacing: Option<&str>) -> AppResult<(SessionInfo, Option<Live>)> {
        let SpawnPlan { id, profile_id, kind, cwd, program, args, env, title, name, place, replaces, claude_session_id, mut request } = plan;
        request.prompt = None;
        // a restarted tab keeps the size its terminal has
        let size = replacing.and_then(|old| self.table().live.get(old).and_then(|l| l.size));
        let Launched { pty, child, stream, exited_tx, exited_rx } = self.launch(&id, &program, &args, &cwd, &env, size, None)?;
        let now = now_ms();

        let mut info = SessionInfo {
            id: id.clone(),
            profile_id: profile_id.clone(),
            kind,
            cwd,
            title,
            status: SessionStatus::Running,
            exit_code: None,
            started_at: now,
            name: if kind == SessionKind::Claude { name } else { None },
            replaces: None,
            place: None,
            claude_session_id,
            model: None,
            context_pct: None,
            turn: None,
            moving: false,
            arrived: None,
            keep_awake: false,
            slept_at: None,
        };
        let old = {
            let mut t = self.table();
            // "Keep awake" stays with the tab through a restart or a move
            info.keep_awake = replacing.and_then(|old| t.live.get(old)).is_some_and(|l| l.info.keep_awake);
            let live = Live {
                info: info.clone(),
                request,
                transcript_path: None,
                pty: Some(pty),
                ending: false,
                stream: stream.clone(),
                exited: exited_rx,
                intro: (place.is_some() || replaces.is_some()).then_some((place, replaces)),
                last_input_at: now,
                size,
                to_sleep: false,
            };
            let at = replacing.and_then(|old| t.order.iter().position(|x| x == old));
            match at {
                Some(i) => t.order.insert(i, id.clone()),
                None => t.order.push(id.clone()),
            }
            t.live.insert(id.clone(), live);
            replacing.and_then(|old| t.remove(old))
        };

        let waiting = self.start_waiter(&id, &profile_id, stream, child, exited_tx);
        if let Err(e) = waiting {
            let gone = self.table().remove(&id);
            if let Some(gone) = gone {
                self.dispose(gone);
            }
            if let Some(old) = old {
                // put the old one back where it was
                let mut t = self.table();
                let restored = old.info.id.clone();
                t.order.push(restored.clone());
                t.live.insert(restored, old);
            }
            return Err(AppError::io(format!("Could not start the session: {e}")));
        }
        Ok((info, old))
    }

    // ---------- sleeping ----------

    /// Puts a running Claude session to sleep: its process is asked to end as [`Self::end`] asks
    /// it (killed after `timeout_ms` at the latest), and the session stays in its tab under its
    /// id as [`SessionStatus::Sleeping`], with its output, its name, its conversation and its
    /// launch options. Its settings file and hook address go as they go for any ended process
    /// (the cleanup handler); the exit handler is not called, since for the user nothing ended.
    ///
    /// Until the process is gone the session carries the moving mark (not announced), so that it
    /// cannot be restarted, moved or woken in between. Refused for anything but a running Claude
    /// session that is not being moved, restarted or put to sleep already. The returned future
    /// resolves once the process is gone, or at `timeout_ms`.
    pub fn sleep(&self, id: &str, timeout_ms: u64) -> AppResult<impl Future<Output = ()> + Send + 'static> {
        {
            let mut t = self.table();
            let live = t.live.get_mut(id).ok_or_else(|| AppError::refused("Session not found"))?;
            if live.info.kind != SessionKind::Claude || live.info.status != SessionStatus::Running || live.pty.is_none() {
                return Err(AppError::refused("Only a running Claude tab can be put to sleep."));
            }
            if live.info.moving || live.to_sleep {
                return Err(AppError::refused("This tab is being moved, restarted or put to sleep already."));
            }
            live.info.moving = true;
            live.to_sleep = true;
        }
        let ending = self.end(id, timeout_ms);
        let manager = self.clone();
        let id = id.to_owned();
        Ok(async move {
            ending.await;
            manager.sleep_requested_done(&id);
        })
    }

    /// The end that [`Self::sleep`] asked for is over or timed out. A process that is still there
    /// sleeps once it ends; until then the mark comes off, so that the tab can be closed or
    /// restarted as usual.
    fn sleep_requested_done(&self, id: &str) {
        let mut t = self.table();
        if let Some(live) = t.live.get_mut(id).filter(|l| l.to_sleep) {
            live.info.moving = false;
        }
    }

    /// A tab that comes back asleep from the last run: it joins the list as
    /// [`SessionStatus::Sleeping`] with an empty replay and no process, without being announced
    /// ([`Self::announce`] does that, as after [`Self::start`]).
    pub fn create_sleeping(&self, tab: SleepingTab) -> SessionInfo {
        let SleepingTab { id, profile_id, cwd, name, claude_session_id, transcript_path, mut request } = tab;
        request.prompt = None;
        let now = now_ms();
        let info = SessionInfo {
            id: id.clone(),
            profile_id,
            kind: SessionKind::Claude,
            title: title_of(&cwd),
            cwd,
            status: SessionStatus::Sleeping,
            exit_code: None,
            started_at: now,
            name,
            replaces: None,
            place: None,
            claude_session_id,
            model: None,
            context_pct: None,
            turn: None,
            moving: false,
            arrived: None,
            keep_awake: false,
            slept_at: Some(now),
        };
        // nothing will ever run in it: its end has come already
        let (_tx, exited) = watch::channel(true);
        let live = Live {
            info: info.clone(),
            request,
            transcript_path,
            pty: None,
            ending: false,
            stream: Stream::new(id.clone(), self.inner.outlet.clone()),
            exited,
            intro: None,
            last_input_at: now,
            size: None,
            to_sleep: false,
        };
        let mut t = self.table();
        if t.live.insert(id.clone(), live).is_none() {
            t.order.push(id);
        }
        info
    }

    /// Starts a sleeping session again, under its own id: `plan` was made for it with
    /// [`Self::plan_as`]. The new process gets the size the terminal last had, and its output
    /// continues the replay and `seq` of the old one, so the window keeps the tab, its terminal
    /// and what it shows. Its name and launch options are the plan's (which keeps those of the
    /// session). On failure the session stays asleep and the plan's files are cleaned up.
    pub fn wake(&self, plan: SpawnPlan) -> AppResult<SessionInfo> {
        let (profile_id, id) = (plan.profile_id.clone(), plan.id.clone());
        match self.wake_inner(plan) {
            Ok(info) => {
                self.emit_changed(None);
                Ok(info)
            }
            Err(e) => {
                self.cleanup(&profile_id, &id);
                Err(e)
            }
        }
    }

    fn wake_inner(&self, plan: SpawnPlan) -> AppResult<SessionInfo> {
        let SpawnPlan { id, profile_id, cwd, program, args, env, title, name, claude_session_id, mut request, .. } = plan;
        request.prompt = None;
        let asleep = |l: &Live| l.info.status == SessionStatus::Sleeping && l.info.profile_id == profile_id;
        let (before, size) = {
            let t = self.table();
            let live = t.live.get(&id).ok_or_else(|| AppError::refused("Session not found"))?;
            if !asleep(live) {
                return Err(AppError::refused("This tab is not asleep."));
            }
            (live.stream.clone(), live.size)
        };
        let Launched { pty, child, stream, exited_tx, exited_rx } = self.launch(&id, &program, &args, &cwd, &env, size, Some(&before))?;
        let now = now_ms();
        let taken = {
            let mut t = self.table();
            match t.live.get_mut(&id).filter(|l| asleep(l)) {
                // closed while the process started
                None => Err(pty),
                Some(live) => {
                    live.request = request;
                    live.pty = Some(pty);
                    live.ending = false;
                    live.to_sleep = false;
                    live.stream = stream.clone();
                    live.exited = exited_rx;
                    live.last_input_at = now;
                    // another conversation than the one it slept in (a new one, or the folder's last)
                    if claude_session_id.is_some() && claude_session_id != live.info.claude_session_id {
                        live.transcript_path = None;
                        live.info.claude_session_id = claude_session_id;
                    }
                    let info = &mut live.info;
                    info.status = SessionStatus::Running;
                    info.exit_code = None;
                    info.started_at = now;
                    info.slept_at = None;
                    info.turn = None;
                    // the mark of whoever woke it: it is awake now, and the list says so
                    info.moving = false;
                    info.title = title;
                    info.name = name;
                    info.cwd = cwd;
                    Ok(info.clone())
                }
            }
        };
        let info = match taken {
            Ok(info) => info,
            Err(mut pty) => {
                stream.close();
                let _ = pty.killer.kill();
                close_later(pty.master.take());
                return Err(AppError::refused("Session not found"));
            }
        };
        if let Err(e) = self.start_waiter(&id, &profile_id, stream.clone(), child, exited_tx) {
            // back to sleep, with the output it had
            let pty = {
                let mut t = self.table();
                t.live.get_mut(&id).filter(|l| Arc::ptr_eq(&l.stream, &stream)).and_then(|live| {
                    live.stream = before;
                    live.info.status = SessionStatus::Sleeping;
                    live.info.slept_at = Some(now_ms());
                    live.pty.take()
                })
            };
            stream.close();
            if let Some(mut pty) = pty {
                let _ = pty.killer.kill();
                close_later(pty.master.take());
            }
            return Err(AppError::io(format!("Could not start the session: {e}")));
        }
        Ok(info)
    }

    /// The waiter thread: the process ended. Its output is let through to the end, then the
    /// session is marked as ended and announced.
    fn wait_exit(&self, id: &str, profile_id: &str, stream: &Arc<Stream>, mut child: Box<dyn Child + Send + Sync>, exited: watch::Sender<bool>) {
        let code = match child.wait() {
            Ok(status) => Some(i64::from(status.exit_code())),
            Err(e) => {
                log::warn!("waiting for session {id}: {e}");
                None
            }
        };
        // Windows: the console hands over the end of the output once it is closed
        if cfg!(windows) {
            drop(self.take_master(id, stream));
        }
        if !stream.wait_end(DRAIN_TIMEOUT) {
            log::debug!("the output of session {id} did not end with its process");
        }
        drop(self.take_master(id, stream));
        self.finish_exit(id, profile_id, stream, code);
        exited.send_replace(true);
    }

    /// The pty of the session `id`, if it is still the one of `stream` and not closed yet.
    fn take_master(&self, id: &str, stream: &Arc<Stream>) -> Option<Box<dyn Console>> {
        let mut t = self.table();
        let live = t.live.get_mut(id).filter(|l| Arc::ptr_eq(&l.stream, stream))?;
        live.pty.as_mut()?.master.take()
    }

    /// The process of a session is gone: it is marked as ended, or as asleep when [`Self::sleep`]
    /// ended it. A session that falls asleep keeps the moving mark until its files are cleaned up,
    /// so that waking it (which makes them anew under the same id) cannot come in between.
    fn finish_exit(&self, id: &str, profile_id: &str, stream: &Arc<Stream>, code: Option<i64>) {
        stream.flush();
        let mine = |l: &&mut Live| Arc::ptr_eq(&l.stream, stream);
        let (mut info, pty, asleep) = {
            let mut t = self.table();
            match t.live.get_mut(id).filter(mine) {
                // closing the tab caused the exit and already removed the session
                None => (None, None, false),
                Some(live) => {
                    let pty = live.pty.take();
                    let asleep = std::mem::take(&mut live.to_sleep);
                    if asleep {
                        live.info.status = SessionStatus::Sleeping;
                        live.info.slept_at = Some(now_ms());
                    } else {
                        live.info.status = SessionStatus::Exited;
                        live.info.exit_code = code;
                    }
                    live.info.turn = None;
                    (Some(live.info.clone()), pty, asleep)
                }
            }
        };
        if let Some(mut pty) = pty {
            close_later(pty.master.take());
        }
        self.cleanup(profile_id, id);
        if asleep {
            let mut t = self.table();
            info = t.live.get_mut(id).filter(mine).map(|live| {
                live.info.moving = false;
                live.info.clone()
            });
        }
        let Some(info) = info else { return };
        self.emit_changed(None);
        if asleep {
            log::info!("session {} is asleep", tag_of(id));
            return;
        }
        let handler = self.handlers().exit.clone();
        if let Some(on_exit) = handler {
            on_exit(info);
        }
    }

    // ---------- the window's calls ----------

    /// Sends what waits, then hands over the session, its replay and the `seq` after it.
    pub fn attach(&self, id: &str) -> Option<AttachResult> {
        let (info, stream) = {
            let t = self.table();
            let live = t.live.get(id)?;
            (live.info.clone(), live.stream.clone())
        };
        let (replay, seq) = stream.attach();
        Some(AttachResult { info, replay, seq })
    }

    /// Writes to the session's terminal; false when it has no running process (an ended or a
    /// sleeping session). What a person typed or pasted, as opposed to the terminal's own reports,
    /// counts as activity for the idle check.
    pub fn write(&self, id: &str, data: &str) -> bool {
        if data.is_empty() {
            return false;
        }
        let mut t = self.table();
        let Some(live) = t.live.get_mut(id) else { return false };
        let sent = live.pty.as_ref().is_some_and(|p| p.input.send(data.as_bytes().to_vec()).is_ok());
        if sent && is_typing(data) {
            live.last_input_at = now_ms();
        }
        sent
    }

    /// Resizes the session's terminal; a size outside 2..=1000 either way is ignored. The size is
    /// kept for a process that starts in the terminal later (a sleeping session that wakes).
    pub fn resize(&self, id: &str, cols: u16, rows: u16) {
        let range = PTY_MIN..=PTY_MAX;
        if !range.contains(&cols) || !range.contains(&rows) {
            return;
        }
        let mut t = self.table();
        let Some(live) = t.live.get_mut(id) else { return };
        live.size = Some((cols, rows));
        if let Some(master) = live.pty.as_ref().and_then(|p| p.master.as_ref()) {
            if let Err(e) = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }) {
                // the process exited between the check and the resize
                log::debug!("resize {id}: {e}");
            }
        }
    }

    /// Tells the session's process to end, once; a second call does nothing.
    pub fn kill(&self, id: &str) {
        let master = self.table().live.get_mut(id).and_then(Live::kill);
        close_later(master);
    }

    /// Ends a session's process and resolves once it is gone, or after `timeout_ms` at the
    /// latest. The tab stays, as an ended session.
    ///
    /// On Windows the console is closed first, so that the program can finish what it was
    /// writing (see `Live::end_gently`); one that is still there after [`END_GRACE`], or at
    /// `timeout_ms` when that comes sooner, is killed. The process is only killed while the
    /// returned future is awaited. Closing a tab and quitting kill at once instead.
    pub fn end(&self, id: &str, timeout_ms: u64) -> impl Future<Output = ()> + Send + 'static {
        let waiting = {
            let mut t = self.table();
            match t.live.get_mut(id) {
                Some(live) if live.pty.is_some() => {
                    let exited = live.exited.clone();
                    Some((exited, live.end_gently()))
                }
                _ => None,
            }
        };
        let exited = waiting.map(|(exited, console)| {
            close_later(console);
            exited
        });
        let manager = self.clone();
        let id = id.to_owned();
        async move {
            let Some(mut exited) = exited else { return };
            let timeout = Duration::from_millis(timeout_ms);
            if !cfg!(windows) {
                exited_within(&mut exited, timeout).await;
                return;
            }
            let grace = timeout.min(END_GRACE);
            if exited_within(&mut exited, grace).await {
                return;
            }
            // it did not end by itself
            manager.kill(&id);
            exited_within(&mut exited, timeout - grace).await;
        }
    }

    pub fn close(&self, id: &str) {
        self.close_many(&[id]);
    }

    /// Closes several tabs with a single announcement.
    pub fn close_many<S: AsRef<str>>(&self, ids: &[S]) {
        let removed: Vec<Live> = {
            let mut t = self.table();
            ids.iter().filter_map(|id| t.remove(id.as_ref())).collect()
        };
        if removed.is_empty() {
            return;
        }
        for live in removed {
            self.dispose(live);
        }
        self.emit_changed(None);
    }

    /// Closes the tabs of an account (it is being deleted).
    pub fn close_for_profile(&self, profile_id: &str) {
        let removed: Vec<Live> = {
            let mut t = self.table();
            let ids: Vec<String> = t.order.iter().filter(|id| t.live.get(*id).is_some_and(|l| l.info.profile_id == profile_id)).cloned().collect();
            ids.iter().filter_map(|id| t.remove(id)).collect()
        };
        for live in removed {
            self.dispose(live);
        }
        self.emit_changed(None);
    }

    /// Ends every session without announcing anything (the app quits).
    pub fn kill_all(&self) {
        let removed: Vec<Live> = {
            let mut t = self.table();
            t.order.clear();
            t.live.drain().map(|(_, live)| live).collect()
        };
        for live in removed {
            self.dispose(live);
        }
    }

    /// `remove` after the session left the list: no more output, the process told to end, the
    /// pty closed, the settings cleaned up. A sleeping session let go of its files when it fell
    /// asleep (a wake on its way cleans up after itself when it finds the tab gone).
    fn dispose(&self, mut live: Live) {
        live.stream.close();
        close_later(live.kill());
        if let Some(mut pty) = live.pty.take() {
            close_later(pty.master.take());
        }
        if live.info.status != SessionStatus::Sleeping {
            self.cleanup(&live.info.profile_id, &live.info.id);
        }
    }

    /// Names a tab; without a name it shows its automatic title again.
    pub fn rename(&self, id: &str, name: Option<String>) {
        let changed = {
            let mut t = self.table();
            match t.live.get_mut(id) {
                Some(live) if live.info.kind == SessionKind::Claude && live.info.name != name => {
                    // the name travels with the request, so a restart keeps it
                    live.request.name.clone_from(&name);
                    live.info.name = name;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_changed(None);
        }
    }

    // ---------- what other services report ----------

    /// What the status line reports about a session: its conversation, model and context use.
    pub fn set_meta(&self, id: &str, meta: &SessionMeta) {
        let changed = {
            let mut t = self.table();
            let Some(live) = t.live.get_mut(id) else { return };
            if let Some(path) = meta.transcript_path.as_ref().filter(|p| !p.is_empty()) {
                live.transcript_path = Some(path.clone());
            }
            let info = &mut live.info;
            // the conversation id changes when the user runs /clear or /resume inside the session
            let claude_session_id = meta.claude_session_id.clone().or_else(|| info.claude_session_id.clone());
            let model = meta.model.clone().or_else(|| info.model.clone());
            let context_pct = meta.context_pct.or(info.context_pct);
            if claude_session_id == info.claude_session_id && model == info.model && context_pct == info.context_pct {
                false
            } else {
                info.claude_session_id = claude_session_id;
                info.model = model;
                info.context_pct = context_pct;
                true
            }
        };
        if changed {
            self.emit_changed(None);
        }
    }

    /// What the session's hooks report about its turn.
    pub fn set_turn(&self, id: &str, turn: Option<Turn>) {
        let changed = {
            let mut t = self.table();
            match t.live.get_mut(id) {
                Some(live) if live.info.status == SessionStatus::Running && live.info.turn != turn => {
                    live.info.turn = turn;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_changed(None);
        }
    }

    /// Marks a session as on its way to another account unless it already is; true when this
    /// call set the mark. Checked and set in one step, so that two moves, or a move and a restart,
    /// cannot both go ahead. Nothing is announced: the caller does that when the window is to
    /// show the mark ([`Self::announce`]); [`Self::set_moving`] takes it off again.
    pub fn try_set_moving(&self, id: &str) -> bool {
        let mut t = self.table();
        match t.live.get_mut(id) {
            Some(live) if !live.info.moving => {
                live.info.moving = true;
                true
            }
            _ => false,
        }
    }

    /// Marks a session as on its way to another account, or takes the mark off again.
    pub fn set_moving(&self, id: &str, moving: bool) {
        let changed = {
            let mut t = self.table();
            match t.live.get_mut(id) {
                Some(live) if live.info.moving != moving => {
                    live.info.moving = moving;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.emit_changed(None);
        }
    }

    /// Notes how a session's conversation came to its account.
    pub fn set_arrived(&self, id: &str, arrived: Arrival) {
        let found = {
            let mut t = self.table();
            t.live.get_mut(id).map(|live| live.info.arrived = Some(arrived)).is_some()
        };
        if found {
            self.emit_changed(None);
        }
    }

    /// The user's "Keep awake": a tab with it is never put to sleep for being idle. Only a Claude
    /// tab takes it; announced when it changes.
    pub fn set_keep_awake(&self, id: &str, on: bool) -> AppResult<()> {
        let changed = {
            let mut t = self.table();
            let live = t.live.get_mut(id).ok_or_else(|| AppError::refused("Session not found"))?;
            if live.info.kind != SessionKind::Claude {
                return Err(AppError::refused("Only a Claude tab can be kept awake."));
            }
            std::mem::replace(&mut live.info.keep_awake, on) != on
        };
        if changed {
            self.emit_changed(None);
        }
        Ok(())
    }

    /// What the idle check needs of every session, in tab order.
    pub fn activity(&self) -> Vec<Activity> {
        let t = self.table();
        t.order
            .iter()
            .filter_map(|id| t.live.get(id))
            .map(|l| Activity { info: l.info.clone(), last_input_at: l.last_input_at, last_output_at: l.stream.last_output_at() })
            .collect()
    }

    // ---------- the list ----------

    /// `announce` / `emitSessions`: sends the list, with `layout` when given (else the changed
    /// handler's). Called from inside the changed handler, it sends once more right after.
    pub fn emit_changed(&self, layout: Option<Layout>) {
        if ANNOUNCING.with(Cell::get) {
            let mut again = lock(&self.inner.again);
            again.pending = true;
            if layout.is_some() {
                again.layout = layout;
            }
            return;
        }
        let _one_at_a_time = lock(&self.inner.announcing);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                ANNOUNCING.with(|a| a.set(false));
            }
        }
        ANNOUNCING.with(|a| a.set(true));
        let _reset = Reset;
        let mut layout = layout;
        loop {
            let sessions = self.snapshot();
            let handler = self.handlers().changed.clone();
            let from_handler = handler.and_then(|changed| changed(&sessions));
            self.inner.outlet.sessions(SessionsChanged { sessions, layout: layout.take().or(from_handler) });
            let mut again = lock(&self.inner.again);
            if !std::mem::take(&mut again.pending) {
                break;
            }
            layout = again.layout.take();
        }
    }

    /// `announce`: sends the list.
    pub fn announce(&self) {
        self.emit_changed(None);
    }

    /// The list as it is sent: a session that is new in it carries its `place` and `replaces`.
    fn snapshot(&self) -> Vec<SessionInfo> {
        let mut t = self.table();
        let Table { live, order } = &mut *t;
        order
            .iter()
            .filter_map(|id| {
                let live = live.get_mut(id)?;
                let mut info = live.info.clone();
                if let Some((place, replaces)) = live.intro.take() {
                    info.place = place;
                    info.replaces = replaces;
                }
                Some(info)
            })
            .collect()
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let t = self.table();
        t.order.iter().filter_map(|id| t.live.get(id)).map(|l| l.info.clone()).collect()
    }

    pub fn has_running(&self) -> bool {
        self.table().live.values().any(|l| l.info.status == SessionStatus::Running)
    }

    pub fn get(&self, id: &str) -> Option<SessionInfo> {
        self.table().live.get(id).map(|l| l.info.clone())
    }

    pub fn cwd_of(&self, id: &str) -> Option<String> {
        self.table().live.get(id).map(|l| l.info.cwd.clone())
    }

    /// What the session was asked to be (without its first message), to start it again.
    pub fn request_of(&self, id: &str) -> Option<SessionRequest> {
        self.table().live.get(id).map(|l| l.request.clone())
    }

    /// How the session's Claude was started, to start it the same way somewhere else.
    pub fn launch_of(&self, id: &str) -> Option<LaunchOptions> {
        self.table().live.get(id).and_then(|l| l.request.launch.clone())
    }

    /// Where Claude Code keeps the session's conversation, once its status line has said so.
    pub fn transcript_of(&self, id: &str) -> Option<String> {
        self.table().live.get(id).and_then(|l| l.transcript_path.clone())
    }

    /// The running and the sleeping Claude tabs, in order, as they are remembered for the next
    /// start; a sleeping one says so (`asleep`).
    pub fn saved_tabs(&self) -> Vec<(String, SavedTab)> {
        let t = self.table();
        t.order
            .iter()
            .filter_map(|id| t.live.get(id))
            .filter(|l| l.info.kind == SessionKind::Claude && matches!(l.info.status, SessionStatus::Running | SessionStatus::Sleeping))
            .map(|l| {
                let tab = SavedTab {
                    profile_id: l.info.profile_id.clone(),
                    cwd: l.info.cwd.clone(),
                    claude_session_id: l.info.claude_session_id.clone(),
                    transcript_path: l.transcript_path.clone(),
                    name: l.info.name.clone(),
                    asleep: l.info.status == SessionStatus::Sleeping,
                };
                (l.info.id.clone(), tab)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
