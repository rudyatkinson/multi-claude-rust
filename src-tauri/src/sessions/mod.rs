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
use mc_core::types::{
    Arrival, AttachResult, HookNeed, LaunchOptions, Layout, Place, PtyData, SavedTab, SessionInfo, SessionKind, SessionRequest,
    SessionStatus, SessionsChanged, Turn,
};
use mc_core::validate::{PTY_MAX, PTY_MIN};
use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use tauri::AppHandle;
use tokio::sync::watch;

use crate::claude_binary::{is_shim, spawn_spec};
use stream::Stream;

/// How long an ended program gets to hand over the rest of its output.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(1000);

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

/// What the status line reports about a session (`SessionMeta` of `src/main/usage/normalize.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    pub claude_session_id: Option<String>,
    /// Where Claude Code keeps the conversation.
    pub transcript_path: Option<String>,
    pub model: Option<String>,
    /// Whole percent of the context window in use.
    pub context_pct: Option<u32>,
}

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
    /// Taken when the pty is closed early (a kill on Windows).
    master: Option<Box<dyn MasterPty + Send>>,
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
}

impl Live {
    /// Tells the process to end, once. On Windows the pseudoconsole is closed as well, which ends
    /// whatever else runs in it (node-pty did both); the caller closes the returned console.
    fn kill(&mut self) -> Option<Box<dyn MasterPty + Send>> {
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
fn close_later(master: Option<Box<dyn MasterPty + Send>>) {
    if let Some(master) = master {
        // if no thread can be started, the console is closed here
        let _ = pty::thread("pty-close".into(), move || drop(master));
    }
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
        check_folder(&request.cwd)?;
        let id = new_session_id();
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

    fn spawn_inner(&self, plan: SpawnPlan, replacing: Option<&str>) -> AppResult<(SessionInfo, Option<Live>)> {
        let SpawnPlan { id, profile_id, kind, cwd, program, args, env, title, name, place, replaces, claude_session_id, mut request } = plan;
        request.prompt = None;
        let opened = pty::open(&program, &args, &cwd, &env).map_err(AppError::io)?;
        let stream = Stream::new(id.clone(), self.inner.outlet.clone());
        let tag: String = id.chars().take(8).collect();
        let started = pty::start(opened, stream.clone(), &tag).map_err(AppError::io)?;
        let pty::Started { master, child, killer, input } = started;
        let (exited_tx, exited_rx) = watch::channel(false);

        let info = SessionInfo {
            id: id.clone(),
            profile_id: profile_id.clone(),
            kind,
            cwd,
            title,
            status: SessionStatus::Running,
            exit_code: None,
            started_at: now_ms(),
            name: if kind == SessionKind::Claude { name } else { None },
            replaces: None,
            place: None,
            claude_session_id,
            model: None,
            context_pct: None,
            turn: None,
            moving: false,
            arrived: None,
        };
        let live = Live {
            info: info.clone(),
            request,
            transcript_path: None,
            pty: Some(Pty { master: Some(master), input, killer }),
            ending: false,
            stream: stream.clone(),
            exited: exited_rx,
            intro: (place.is_some() || replaces.is_some()).then_some((place, replaces)),
        };
        let old = {
            let mut t = self.table();
            let at = replacing.and_then(|old| t.order.iter().position(|x| x == old));
            match at {
                Some(i) => t.order.insert(i, id.clone()),
                None => t.order.push(id.clone()),
            }
            t.live.insert(id.clone(), live);
            replacing.and_then(|old| t.remove(old))
        };

        // the waiter goes last: the session must be in the list when its process ends
        let manager = self.clone();
        let waiter_stream = stream;
        let (wait_id, wait_profile) = (id.clone(), profile_id);
        let waiting = pty::thread(format!("pty-wait-{tag}"), move || {
            manager.wait_exit(&wait_id, &wait_profile, &waiter_stream, child, exited_tx);
        });
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
    fn take_master(&self, id: &str, stream: &Arc<Stream>) -> Option<Box<dyn MasterPty + Send>> {
        let mut t = self.table();
        let live = t.live.get_mut(id).filter(|l| Arc::ptr_eq(&l.stream, stream))?;
        live.pty.as_mut()?.master.take()
    }

    fn finish_exit(&self, id: &str, profile_id: &str, stream: &Arc<Stream>, code: Option<i64>) {
        stream.flush();
        let (info, pty) = {
            let mut t = self.table();
            match t.live.get_mut(id).filter(|l| Arc::ptr_eq(&l.stream, stream)) {
                // closing the tab caused the exit and already removed the session
                None => (None, None),
                Some(live) => {
                    let pty = live.pty.take();
                    live.info.status = SessionStatus::Exited;
                    live.info.exit_code = code;
                    live.info.turn = None;
                    (Some(live.info.clone()), pty)
                }
            }
        };
        if let Some(mut pty) = pty {
            close_later(pty.master.take());
        }
        self.cleanup(profile_id, id);
        let Some(info) = info else { return };
        self.emit_changed(None);
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

    /// Writes to the session's terminal; false when it has no running process.
    pub fn write(&self, id: &str, data: &str) -> bool {
        if data.is_empty() {
            return false;
        }
        let t = self.table();
        t.live.get(id).and_then(|l| l.pty.as_ref()).is_some_and(|p| p.input.send(data.as_bytes().to_vec()).is_ok())
    }

    /// Resizes the session's terminal; a size outside 2..=1000 either way is ignored.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) {
        let range = PTY_MIN..=PTY_MAX;
        if !range.contains(&cols) || !range.contains(&rows) {
            return;
        }
        let t = self.table();
        if let Some(master) = t.live.get(id).and_then(|l| l.pty.as_ref()).and_then(|p| p.master.as_ref()) {
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
    pub fn end(&self, id: &str, timeout_ms: u64) -> impl Future<Output = ()> + Send + 'static {
        let waiting = {
            let mut t = self.table();
            match t.live.get_mut(id) {
                Some(live) if live.pty.is_some() => {
                    let exited = live.exited.clone();
                    Some((exited, live.kill()))
                }
                _ => None,
            }
        };
        let exited = waiting.map(|(exited, master)| {
            close_later(master);
            exited
        });
        async move {
            let Some(mut exited) = exited else { return };
            let gone = async move {
                while !*exited.borrow_and_update() {
                    if exited.changed().await.is_err() {
                        break;
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_millis(timeout_ms), gone).await;
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
    /// pty closed, the settings cleaned up.
    fn dispose(&self, mut live: Live) {
        live.stream.close();
        close_later(live.kill());
        if let Some(mut pty) = live.pty.take() {
            close_later(pty.master.take());
        }
        self.cleanup(&live.info.profile_id, &live.info.id);
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

    /// The running Claude tabs, in order, as they are remembered for the next start.
    pub fn saved_tabs(&self) -> Vec<(String, SavedTab)> {
        let t = self.table();
        t.order
            .iter()
            .filter_map(|id| t.live.get(id))
            .filter(|l| l.info.kind == SessionKind::Claude && l.info.status == SessionStatus::Running)
            .map(|l| {
                let tab = SavedTab {
                    profile_id: l.info.profile_id.clone(),
                    cwd: l.info.cwd.clone(),
                    claude_session_id: l.info.claude_session_id.clone(),
                    transcript_path: l.transcript_path.clone(),
                    name: l.info.name.clone(),
                };
                (l.info.id.clone(), tab)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
