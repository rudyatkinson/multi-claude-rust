//! Notifications when a session stops and waits (`src/main/sessionAlerts.ts`, and the `tell` of
//! `src/main/index.ts` that shows them).
//!
//! Decides when a session that calls for attention is announced, and as what. The window reports
//! such a session twice at times, once for its title going quiet and once for what its hooks say;
//! each thing a turn comes to is announced once. An announcement is a flash of the window's taskbar
//! button and a desktop notification; a click on the notification brings the window to the front
//! and its tab with it.
//!
//! What the service needs of the rest of the app comes in through [`AlertDeps`], set with
//! [`SessionAlerts::set_deps`] at integration; until then nothing is announced.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError, RwLock};
use std::time::Duration;

use mc_core::turn::{stop_sentence, turn_key};
use mc_core::types::{Activate, SessionInfo, SessionStatus, Turn, TurnKind, TurnWait};
use tauri::{AppHandle, Manager};

use crate::events;

/// The terminal title can change a moment before the hook arrives that says why; it is waited for this long.
pub const HOOK_GRACE_MS: u64 = 400;

/// What the alerts need to know of the rest of the app.
pub trait AlertDeps: Send + Sync {
    /// The session of that id while it exists. Its `turn`, `title` and `profile_id` are read.
    fn session(&self, id: &str) -> Option<SessionInfo>;
    /// The name of a profile (account), while it exists.
    fn profile_name(&self, profile_id: &str) -> Option<String>;
    /// `settings.notifications !== false`.
    fn notifications_on(&self) -> bool;
    /// A workflow run watches the session (`workflowRuns.watches`): a step reports for itself and
    /// says what the run waits for.
    fn is_workflow_step(&self, id: &str) -> bool;
}

/// How an announcement reaches the user. The app's own is [`TauriOutput`]; tests put a fake here.
pub trait AlertOutput: Send + Sync {
    /// Whether the main window has the keyboard focus; `None` when there is no window.
    fn window_focused(&self) -> Option<bool>;
    /// Electron's `flashFrame(true)`: the taskbar button asks for attention until the window is focused.
    fn flash(&self);
    /// Shows a desktop notification about a session. A click on it brings the window to the front
    /// and activates the session's tab.
    fn notify(&self, session_id: &str, title: &str, body: &str);
}

/// Runs a task later. The app's own is [`TokioTimer`]; tests use a fake clock.
pub trait AlertTimer: Send + Sync {
    fn after(&self, delay: Duration, task: Box<dyn FnOnce() + Send>);
}

/// What to say about a session that calls for attention, for the account it runs on.
pub fn alert_text(account: &str, turn: Option<&Turn>) -> String {
    match turn.map(|t| &t.kind) {
        Some(TurnKind::Permission) => format!("{account} asks for a permission"),
        Some(TurnKind::Question) => format!("{account} asks a question"),
        Some(TurnKind::Plan) => format!("{account} presents a plan and waits for your approval"),
        Some(TurnKind::Done { background: true }) => format!("{account} finished its answer; work it started still runs"),
        Some(TurnKind::Done { background: false }) => format!("{account} finished its answer"),
        Some(TurnKind::Failed { error, text, limit, wait }) => {
            if !*limit {
                return format!("{account}: {}", stop_sentence(*error, text.as_deref().unwrap_or("")));
            }
            match wait {
                Some(TurnWait::NeedsEnter) => format!("{account}: the usage limit has reset. Press Enter in the tab."),
                Some(TurnWait::Ended) => format!("{account}: Claude Code no longer waits for the usage limit."),
                None => format!("{account} stopped at a usage limit"),
            }
        }
        // no hooks, or none that explains why the title went quiet
        Some(TurnKind::Working) | None => format!("{account} is waiting for your input"),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn is_working(turn: Option<&Turn>) -> bool {
    matches!(turn, Some(Turn { kind: TurnKind::Working, .. }))
}

/// A wait for the hook that explains a quiet title.
struct Pending {
    token: u64,
    title: String,
}

#[derive(Default)]
struct AlertState {
    /// session id → the key of the turn it was last announced for
    told: HashMap<String, String>,
    waiting: HashMap<String, Pending>,
    next_token: u64,
}

struct Core {
    state: Mutex<AlertState>,
    deps: RwLock<Option<Arc<dyn AlertDeps>>>,
    output: Arc<dyn AlertOutput>,
    timer: Arc<dyn AlertTimer>,
}

impl Core {
    fn deps(&self) -> Option<Arc<dyn AlertDeps>> {
        self.deps.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn turn_of(&self, id: &str) -> Option<Turn> {
        self.deps()?.session(id)?.turn
    }

    /// The grace wait for a session is over: announce what its turn has come to by now.
    fn fire(&self, id: &str, token: u64) {
        let title = {
            let mut state = lock(&self.state);
            match state.waiting.get(id) {
                Some(pending) if pending.token == token => state.waiting.remove(id).map(|p| p.title),
                _ => None,
            }
        };
        if let Some(title) = title {
            let turn = self.turn_of(id);
            self.tell(id, &title, turn.as_ref());
        }
    }

    fn tell(&self, id: &str, title: &str, turn: Option<&Turn>) {
        if let Some(turn) = turn.filter(|t| t.kind != TurnKind::Working) {
            let key = turn_key(turn);
            let mut state = lock(&self.state);
            if state.told.get(id) == Some(&key) {
                return;
            }
            state.told.insert(id.to_string(), key);
        }
        // the `tell` of index.ts
        let Some(deps) = self.deps() else { return };
        if !deps.notifications_on() || self.output.window_focused() != Some(false) {
            return;
        }
        if deps.is_workflow_step(id) {
            return;
        }
        // a tab that has just fallen asleep has nothing to say
        let Some(info) = deps.session(id).filter(|s| s.status != SessionStatus::Sleeping) else { return };
        self.output.flash();
        let account = deps.profile_name(&info.profile_id).unwrap_or_else(|| "Claude".to_string());
        let title = if title.is_empty() { info.title.as_str() } else { title };
        self.output.notify(id, title, &alert_text(&account, turn));
    }
}

pub struct SessionAlerts {
    core: Arc<Core>,
}

impl SessionAlerts {
    pub fn new(app: AppHandle) -> Self {
        Self::with_parts(Arc::new(TauriOutput::new(app)), Arc::new(TokioTimer))
    }

    /// The alerts with another output and timer than the app's own (tests).
    pub fn with_parts(output: Arc<dyn AlertOutput>, timer: Arc<dyn AlertTimer>) -> Self {
        SessionAlerts {
            core: Arc::new(Core { state: Mutex::default(), deps: RwLock::new(None), output, timer }),
        }
    }

    /// What the alerts need to know of the rest of the app. Replaces what was set before.
    pub fn set_deps(&self, deps: Arc<dyn AlertDeps>) {
        *self.core.deps.write().unwrap_or_else(PoisonError::into_inner) = Some(deps);
    }

    /// `session:attention`: the window says a session in the background finished its work or asks
    /// for something. `title` is the tab's label. While the hooks still say "working", the one that
    /// says otherwise may be on its way: it is waited for [`HOOK_GRACE_MS`].
    pub fn attention(&self, id: &str, title: &str) {
        let turn = self.core.turn_of(id);
        if !is_working(turn.as_ref()) {
            self.core.tell(id, title, turn.as_ref());
            return;
        }
        let token = {
            let mut state = lock(&self.core.state);
            if state.waiting.contains_key(id) {
                return;
            }
            state.next_token += 1;
            let token = state.next_token;
            state.waiting.insert(id.to_string(), Pending { token, title: title.to_string() });
            token
        };
        let core = Arc::clone(&self.core);
        let id = id.to_string();
        self.core.timer.after(Duration::from_millis(HOOK_GRACE_MS), Box::new(move || core.fire(&id, token)));
    }

    /// A session's turn changed. When a grace wait is running for it and the hooks no longer say
    /// "working", the wait is cut short: what it waited for has come. Not in the TypeScript, where
    /// the wait always ran its 400 ms; what is announced is the same, only sooner.
    pub fn turn_changed(&self, info: &SessionInfo) {
        let Some(turn) = info.turn.as_ref().filter(|t| t.kind != TurnKind::Working) else { return };
        let Some(pending) = lock(&self.core.state).waiting.remove(&info.id) else { return };
        self.core.tell(&info.id, &pending.title, Some(turn));
    }

    /// Forgets the sessions that are gone, with what was still waiting for them. A tab that comes
    /// back under the same id is a new one.
    pub fn prune<I, S>(&self, live_ids: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let live: HashSet<String> = live_ids.into_iter().map(|id| id.as_ref().to_string()).collect();
        let mut state = lock(&self.core.state);
        state.told.retain(|id, _| live.contains(id));
        state.waiting.retain(|id, _| live.contains(id));
    }
}

// ---------- the app's own output and timer ----------

/// Runs a task on the Tauri async runtime after a delay.
pub struct TokioTimer;

impl AlertTimer for TokioTimer {
    fn after(&self, delay: Duration, task: Box<dyn FnOnce() + Send>) {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            task();
        });
    }
}

/// How many shown notifications a click is still traced back from.
const CLICKS_KEPT: usize = 64;

#[derive(Default)]
struct Clicks {
    session_of: HashMap<i32, String>,
    order: VecDeque<i32>,
}

/// The main window and the notification plugin.
pub struct TauriOutput {
    app: AppHandle,
    clicks: Arc<Mutex<Clicks>>,
    listening: Once,
}

impl TauriOutput {
    pub fn new(app: AppHandle) -> Self {
        TauriOutput { app, clicks: Arc::default(), listening: Once::new() }
    }

    fn window(&self) -> Option<tauri::WebviewWindow> {
        self.app.get_webview_window("main")
    }

    /// Registers, once, what a click on one of our notifications does. The plugin reports a click
    /// only for notifications shown after a handler exists, so this runs before the first one.
    fn listen_for_clicks(&self) {
        self.listening.call_once(|| {
            let Some(notification) = self.app.try_state::<tauri_plugin_notification::Notification<tauri::Wry>>() else {
                log::warn!("notification plugin missing: clicks on session alerts do nothing");
                return;
            };
            let app = self.app.clone();
            let clicks = Arc::clone(&self.clicks);
            let _ = notification.on_action(move |performed| {
                if !matches!(performed.action_id(), "tap" | "default") {
                    return;
                }
                let Some(shown) = performed.notification() else { return };
                let Some(id) = lock(&clicks).session_of.get(&shown.id()).cloned() else { return };
                // as from the tray: a first appearance also lets the restored tabs through, and a
                // refused focus change flashes the taskbar button
                if let Some(state) = crate::app::state(&app) {
                    crate::app::window::bring_to_front(&state);
                }
                events::emit(&app, events::SESSIONS_ACTIVATE, Activate { id });
            });
        });
    }
}

impl AlertOutput for TauriOutput {
    fn window_focused(&self) -> Option<bool> {
        self.window().map(|w| w.is_focused().unwrap_or(false))
    }

    fn flash(&self) {
        if let Some(win) = self.window() {
            let _ = win.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
    }

    fn notify(&self, session_id: &str, title: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        self.listen_for_clicks();
        let id = rand::random::<i32>() & i32::MAX;
        {
            let mut clicks = lock(&self.clicks);
            clicks.session_of.insert(id, session_id.to_string());
            clicks.order.push_back(id);
            while clicks.order.len() > CLICKS_KEPT {
                if let Some(old) = clicks.order.pop_front() {
                    clicks.session_of.remove(&old);
                }
            }
        }
        if self.app.try_state::<tauri_plugin_notification::Notification<tauri::Wry>>().is_none() {
            return;
        }
        if let Err(e) = self.app.notification().builder().id(id).title(title).body(body).show() {
            log::warn!("session alert: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use mc_core::types::{SessionKind, SessionStatus, StopError};

    use super::*;

    // ---------- fakes ----------

    type Task = Box<dyn FnOnce() + Send>;

    #[derive(Default)]
    struct FakeClock {
        now: Mutex<u64>,
        tasks: Mutex<Vec<(u64, Task)>>,
    }

    impl AlertTimer for FakeClock {
        fn after(&self, delay: Duration, task: Task) {
            let due = *lock(&self.now) + delay.as_millis() as u64;
            lock(&self.tasks).push((due, task));
        }
    }

    impl FakeClock {
        fn advance(&self, ms: u64) {
            let now = {
                let mut now = lock(&self.now);
                *now += ms;
                *now
            };
            loop {
                let due = {
                    let mut tasks = lock(&self.tasks);
                    let Some(i) = tasks.iter().position(|(at, _)| *at <= now) else { break };
                    tasks.remove(i).1
                };
                due();
            }
        }
    }

    struct FakeDeps {
        turns: Mutex<HashMap<String, Turn>>,
        gone: Mutex<HashSet<String>>,
        steps: Mutex<HashSet<String>>,
        asleep: Mutex<HashSet<String>>,
        notifications: AtomicBool,
        accounts: bool,
    }

    impl FakeDeps {
        fn new() -> Self {
            FakeDeps {
                turns: Mutex::default(),
                gone: Mutex::default(),
                steps: Mutex::default(),
                asleep: Mutex::default(),
                notifications: AtomicBool::new(true),
                accounts: true,
            }
        }
        fn set(&self, id: &str, turn: Turn) {
            lock(&self.turns).insert(id.to_string(), turn);
        }
    }

    fn info(id: &str, turn: Option<Turn>) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            profile_id: "p1".into(),
            kind: SessionKind::Claude,
            cwd: "/work".into(),
            title: format!("info-{id}"),
            status: SessionStatus::Running,
            exit_code: None,
            started_at: 1,
            name: None,
            replaces: None,
            place: None,
            claude_session_id: None,
            model: None,
            context_pct: None,
            turn,
            moving: false,
            arrived: None,
            keep_awake: false,
            slept_at: None,
        }
    }

    impl AlertDeps for FakeDeps {
        fn session(&self, id: &str) -> Option<SessionInfo> {
            if lock(&self.gone).contains(id) {
                return None;
            }
            let mut info = info(id, lock(&self.turns).get(id).cloned());
            if lock(&self.asleep).contains(id) {
                info.status = SessionStatus::Sleeping;
            }
            Some(info)
        }
        fn profile_name(&self, profile_id: &str) -> Option<String> {
            (self.accounts && profile_id == "p1").then(|| "Main".to_string())
        }
        fn notifications_on(&self) -> bool {
            self.notifications.load(Ordering::SeqCst)
        }
        fn is_workflow_step(&self, id: &str) -> bool {
            lock(&self.steps).contains(id)
        }
    }

    struct FakeOutput {
        focused: Mutex<Option<bool>>,
        told: Mutex<Vec<String>>,
        flashes: AtomicUsize,
    }

    impl AlertOutput for FakeOutput {
        fn window_focused(&self) -> Option<bool> {
            *lock(&self.focused)
        }
        fn flash(&self) {
            self.flashes.fetch_add(1, Ordering::SeqCst);
        }
        fn notify(&self, session_id: &str, title: &str, body: &str) {
            lock(&self.told).push(format!("{session_id}/{title}: {body}"));
        }
    }

    struct Rig {
        alerts: SessionAlerts,
        deps: Arc<FakeDeps>,
        output: Arc<FakeOutput>,
        clock: Arc<FakeClock>,
    }

    impl Rig {
        fn new() -> Self {
            Self::with_deps(FakeDeps::new())
        }
        fn with_deps(deps: FakeDeps) -> Self {
            let output = Arc::new(FakeOutput { focused: Mutex::new(Some(false)), told: Mutex::default(), flashes: AtomicUsize::new(0) });
            let clock = Arc::new(FakeClock::default());
            let alerts = SessionAlerts::with_parts(output.clone(), clock.clone());
            let deps = Arc::new(deps);
            alerts.set_deps(deps.clone());
            Rig { alerts, deps, output, clock }
        }
        fn told(&self) -> Vec<String> {
            lock(&self.output.told).clone()
        }
    }

    fn t(at: i64, kind: TurnKind) -> Turn {
        Turn { at, kind }
    }
    fn limit() -> Turn {
        t(5, TurnKind::Failed { error: StopError::RateLimit, text: None, limit: true, wait: None })
    }
    fn limit_wait(wait: TurnWait) -> Turn {
        t(5, TurnKind::Failed { error: StopError::RateLimit, text: None, limit: true, wait: Some(wait) })
    }

    // ---------- alertText ----------

    #[test]
    fn alert_text_says_what_the_account_is_doing() {
        let text = |turn: Option<Turn>| alert_text("Main", turn.as_ref());
        assert_eq!(text(None), "Main is waiting for your input");
        assert_eq!(text(Some(t(1, TurnKind::Working))), "Main is waiting for your input");
        assert_eq!(text(Some(t(1, TurnKind::Permission))), "Main asks for a permission");
        assert_eq!(text(Some(t(1, TurnKind::Question))), "Main asks a question");
        assert_eq!(text(Some(t(1, TurnKind::Plan))), "Main presents a plan and waits for your approval");
        assert_eq!(text(Some(t(1, TurnKind::Done { background: false }))), "Main finished its answer");
        assert_eq!(text(Some(t(1, TurnKind::Done { background: true }))), "Main finished its answer; work it started still runs");
        assert_eq!(text(Some(limit())), "Main stopped at a usage limit");
        assert_eq!(text(Some(limit_wait(TurnWait::NeedsEnter))), "Main: the usage limit has reset. Press Enter in the tab.");
        assert_eq!(text(Some(limit_wait(TurnWait::Ended))), "Main: Claude Code no longer waits for the usage limit.");
        let signed_out = t(1, TurnKind::Failed { error: StopError::AuthenticationFailed, text: None, limit: false, wait: None });
        assert_eq!(text(Some(signed_out)), "Main: The account is no longer signed in.");
    }

    // ---------- SessionAlerts ----------

    #[test]
    fn says_the_old_sentence_at_once_for_a_session_without_hooks_every_time() {
        let rig = Rig::new();
        rig.alerts.attention("t1", "Fix the build");
        rig.alerts.attention("t1", "Fix the build");
        assert_eq!(rig.told(), ["t1/Fix the build: Main is waiting for your input", "t1/Fix the build: Main is waiting for your input"]);
        assert_eq!(rig.output.flashes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn says_nothing_about_a_sleeping_tab() {
        let rig = Rig::new();
        lock(&rig.deps.asleep).insert("t1".into());
        // its title went quiet as its process ended
        rig.alerts.attention("t1", "Fix the build");
        assert!(rig.told().is_empty());
        assert_eq!(rig.output.flashes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn says_what_the_hooks_have_reported_once_for_each_thing_a_turn_comes_to() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Permission));
        rig.alerts.attention("t1", "Tab");
        // the title went quiet a moment later and the window reports the same thing again
        rig.alerts.attention("t1", "Tab");
        assert_eq!(rig.told(), ["t1/Tab: Main asks for a permission"]);

        rig.deps.set("t1", t(20, TurnKind::Done { background: false }));
        rig.alerts.attention("t1", "Tab");
        rig.deps.set("t1", t(30, TurnKind::Permission));
        rig.alerts.attention("t1", "Tab");
        assert_eq!(rig.told()[1..], ["t1/Tab: Main finished its answer", "t1/Tab: Main asks for a permission"]);
    }

    #[test]
    fn waits_a_moment_for_the_hook_when_the_title_was_first() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.alerts.attention("t1", "Tab");
        assert!(rig.told().is_empty());
        rig.deps.set("t1", t(11, TurnKind::Done { background: false }));
        rig.clock.advance(HOOK_GRACE_MS - 1);
        assert!(rig.told().is_empty());
        rig.clock.advance(1);
        assert_eq!(rig.told(), ["t1/Tab: Main finished its answer"]);
    }

    #[test]
    fn does_not_say_it_twice_when_the_hook_and_the_window_both_come_during_the_wait() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.alerts.attention("t1", "Tab");
        rig.deps.set("t1", t(11, TurnKind::Question));
        rig.alerts.attention("t1", "Tab");
        assert_eq!(rig.told(), ["t1/Tab: Main asks a question"]);
        rig.clock.advance(HOOK_GRACE_MS);
        assert_eq!(rig.told().len(), 1);
    }

    #[test]
    fn falls_back_to_the_old_sentence_when_no_hook_explains_the_quiet_title() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.alerts.attention("t1", "Tab");
        rig.alerts.attention("t1", "Tab");
        rig.clock.advance(HOOK_GRACE_MS);
        assert_eq!(rig.told(), ["t1/Tab: Main is waiting for your input"]);
    }

    #[test]
    fn announces_how_a_wait_for_a_limit_ended_as_news_of_its_own() {
        let rig = Rig::new();
        rig.deps.set("t1", limit());
        rig.alerts.attention("t1", "Tab");
        rig.deps.set("t1", limit_wait(TurnWait::NeedsEnter));
        rig.alerts.attention("t1", "Tab");
        rig.alerts.attention("t1", "Tab");
        assert_eq!(rig.told(), ["t1/Tab: Main stopped at a usage limit", "t1/Tab: Main: the usage limit has reset. Press Enter in the tab."]);
    }

    #[test]
    fn forgets_sessions_that_are_gone_with_what_was_still_waiting_for_them() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.deps.set("t2", t(10, TurnKind::Plan));
        rig.alerts.attention("t1", "One");
        rig.alerts.attention("t2", "Two");
        rig.alerts.prune(["t2"]);
        rig.clock.advance(HOOK_GRACE_MS);
        assert_eq!(rig.told(), ["t2/Two: Main presents a plan and waits for your approval"]);
        // a tab that comes back under the same id is a new one
        rig.alerts.prune(Vec::<String>::new());
        rig.alerts.attention("t2", "Two");
        assert_eq!(rig.told().len(), 2);
    }

    #[test]
    fn a_wait_cut_off_by_prune_does_not_touch_the_next_one() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.alerts.attention("t1", "Old");
        rig.alerts.prune(Vec::<&str>::new());
        rig.clock.advance(HOOK_GRACE_MS / 2);
        rig.alerts.attention("t1", "New");
        // the first timer runs out now, and must not end the second wait
        rig.clock.advance(HOOK_GRACE_MS / 2);
        assert!(rig.told().is_empty());
        rig.clock.advance(HOOK_GRACE_MS / 2);
        assert_eq!(rig.told(), ["t1/New: Main is waiting for your input"]);
    }

    #[test]
    fn a_hook_that_arrives_during_the_wait_ends_it_at_once() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Working));
        rig.alerts.attention("t1", "Tab");
        // still working: nothing to say yet
        rig.alerts.turn_changed(&info("t1", Some(t(10, TurnKind::Working))));
        assert!(rig.told().is_empty());
        let done = t(11, TurnKind::Done { background: false });
        rig.deps.set("t1", done.clone());
        rig.alerts.turn_changed(&info("t1", Some(done)));
        assert_eq!(rig.told(), ["t1/Tab: Main finished its answer"]);
        rig.clock.advance(HOOK_GRACE_MS);
        rig.alerts.attention("t1", "Tab");
        assert_eq!(rig.told().len(), 1);
        // without a wait, a changed turn says nothing by itself
        rig.alerts.turn_changed(&info("t1", Some(t(12, TurnKind::Question))));
        assert_eq!(rig.told().len(), 1);
    }

    #[test]
    fn stays_quiet_while_the_window_is_in_front_or_notifications_are_off() {
        let rig = Rig::new();
        *lock(&rig.output.focused) = Some(true);
        rig.alerts.attention("t1", "Tab");
        *lock(&rig.output.focused) = None;
        rig.alerts.attention("t1", "Tab");
        *lock(&rig.output.focused) = Some(false);
        rig.deps.notifications.store(false, Ordering::SeqCst);
        rig.alerts.attention("t1", "Tab");
        assert!(rig.told().is_empty());
        assert_eq!(rig.output.flashes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_turn_counts_as_told_even_when_nothing_was_shown() {
        let rig = Rig::new();
        rig.deps.set("t1", t(10, TurnKind::Question));
        *lock(&rig.output.focused) = Some(true);
        rig.alerts.attention("t1", "Tab");
        *lock(&rig.output.focused) = Some(false);
        rig.alerts.attention("t1", "Tab");
        assert!(rig.told().is_empty());
    }

    #[test]
    fn leaves_workflow_steps_and_gone_sessions_alone() {
        let rig = Rig::new();
        lock(&rig.deps.steps).insert("step".into());
        rig.alerts.attention("step", "Tab");
        lock(&rig.deps.gone).insert("gone".into());
        rig.alerts.attention("gone", "Tab");
        assert!(rig.told().is_empty());
    }

    #[test]
    fn names_the_tab_and_the_account_as_well_as_it_can() {
        let rig = Rig::with_deps(FakeDeps { accounts: false, ..FakeDeps::new() });
        rig.alerts.attention("t1", "");
        assert_eq!(rig.told(), ["t1/info-t1: Claude is waiting for your input"]);
    }

    #[test]
    fn says_nothing_before_it_knows_the_app() {
        let output = Arc::new(FakeOutput { focused: Mutex::new(Some(false)), told: Mutex::default(), flashes: AtomicUsize::new(0) });
        let alerts = SessionAlerts::with_parts(output.clone(), Arc::new(FakeClock::default()));
        alerts.attention("t1", "Tab");
        assert!(lock(&output.told).is_empty());
    }

    #[test]
    fn the_app_timer_runs_its_task_later() {
        let (tx, rx) = std::sync::mpsc::channel();
        TokioTimer.after(Duration::from_millis(20), Box::new(move || tx.send(()).unwrap()));
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }
}
