//! Moving a conversation to another account by itself when a usage limit cut its turn off
//! (`AutoSwitch` of `src/main/autoSwitch.ts`). The rules are in `mc_core::auto_switch::decide_switch`;
//! this carries them out: it fetches the numbers the decision asks for, one decision at a time,
//! and drops a decision when the tab has moved on meanwhile.
//!
//! What it needs of the rest of the app comes in through [`SwitchDeps`]; the app's own is
//! [`AppSwitch`]. Tests put a fake there and run on a paused tokio clock.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mc_core::auto_switch::decide_switch;
use mc_core::turn::in_progress;
use mc_core::types::{
    LoginState, NoticeLevel, SessionKind, SessionStatus, StayWhy, SwitchDecision, SwitchInput, SwitchMove, SwitchProfile, SwitchSession,
    Turn, TurnEvent, TurnKind, UsageState, SWITCH_RULES,
};
use tauri::AppHandle;
use tokio::sync::watch;

use crate::app::{notice, now_ms, state as app_state, window};
use crate::mcp_servers::BoxFuture;
use crate::state::AppState;

/// A conversation that arrived asks Claude to go on; when that message is not sent within this
/// time, something is in its way.
pub const PICK_UP_WATCH_MS: u64 = 20_000;

/// An account as a decision needs it, with the name the user knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedProfile {
    pub profile: SwitchProfile,
    pub name: String,
}

/// What the auto switch needs of the rest of the app (`AutoSwitchDeps`).
pub trait SwitchDeps: Send + Sync + 'static {
    fn now(&self) -> i64;
    /// The setting is on, and Claude can be handed a first message.
    fn enabled(&self) -> bool;
    /// A running Claude session as a decision sees it.
    fn session(&self, session_id: &str) -> Option<SwitchSession>;
    fn profiles(&self) -> Vec<NamedProfile>;
    fn usage(&self) -> BTreeMap<String, UsageState>;
    /// Asks for an account's usage anew; true when numbers came.
    fn refresh_usage(&self, profile_id: &str) -> BoxFuture<'static, bool>;
    /// Moves the conversation of a tab to another account; gives the id of the tab it continues
    /// in, or why it could not.
    fn move_to(&self, session_id: &str, to_profile_id: &str) -> BoxFuture<'static, Result<String, String>>;
    fn tell(&self, session_id: &str, level: NoticeLevel, message: &str);
    /// Runs a task in the background (the app's async runtime; the test's in tests).
    fn spawn(&self, task: BoxFuture<'static, ()>);
}

/// The failed turn a waiting decision is about, and whether the user typed since.
#[derive(Debug, Clone, Copy)]
struct Asked {
    at: i64,
    typed: bool,
}

struct Job {
    session_id: String,
    failed_at: i64,
    was_busy: bool,
}

#[derive(Default)]
struct Inner {
    moves: Vec<SwitchMove>,
    asked: HashMap<String, Asked>,
    queue: VecDeque<Job>,
    draining: bool,
    /// Pick-up watches started before the last [`AutoSwitch::stop`] carry an older one.
    generation: u64,
}

struct Shared {
    deps: Arc<dyn SwitchDeps>,
    state: Mutex<Inner>,
    /// Decisions asked for and not made yet.
    pending: watch::Sender<usize>,
}

/// Cloning gives another handle to the same auto switch.
#[derive(Clone)]
pub struct AutoSwitch {
    shared: Arc<Shared>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AutoSwitch {
    pub fn new(deps: Arc<dyn SwitchDeps>) -> Self {
        let (pending, _) = watch::channel(0);
        AutoSwitch { shared: Arc::new(Shared { deps, state: Mutex::default(), pending }) }
    }

    fn deps(&self) -> &dyn SwitchDeps {
        &*self.shared.deps
    }

    fn state(&self) -> MutexGuard<'_, Inner> {
        lock(&self.shared.state)
    }

    /// A usage limit cut a session's turn off. `was_busy`: Claude was at work when it happened.
    pub fn cut(&self, session_id: &str, was_busy: bool) {
        let deps = self.deps();
        let Some(turn) = deps.session(session_id).and_then(|s| s.turn) else { return };
        if !matches!(turn.kind, TurnKind::Failed { .. }) || !deps.enabled() {
            return;
        }
        let start = {
            let mut state = self.state();
            state.asked.insert(session_id.to_owned(), Asked { at: turn.at, typed: false });
            state.queue.push_back(Job { session_id: session_id.to_owned(), failed_at: turn.at, was_busy });
            !std::mem::replace(&mut state.draining, true)
        };
        self.shared.pending.send_modify(|n| *n += 1);
        if start {
            let me = self.clone();
            deps.spawn(Box::pin(async move { me.drain().await }));
        }
    }

    /// The user typed into a tab: whatever is being decided about it is theirs now.
    pub fn typed(&self, session_id: &str) {
        if let Some(asked) = self.state().asked.get_mut(session_id) {
            asked.typed = true;
        }
    }

    /// Resolves when every decision that was asked for has been made.
    pub async fn settled(&self) {
        let mut pending = self.shared.pending.subscribe();
        let _ = pending.wait_for(|n| *n == 0).await;
    }

    /// The watches for a task that was not picked up end without a word: the app is on its way out.
    pub fn stop(&self) {
        self.state().generation += 1;
    }

    /// Makes the decisions one at a time, in the order they were asked for.
    async fn drain(&self) {
        loop {
            let job = {
                let mut state = self.state();
                match state.queue.pop_front() {
                    Some(job) => job,
                    None => {
                        state.draining = false;
                        return;
                    }
                }
            };
            self.decide(&job).await;
            {
                let mut state = self.state();
                if state.asked.get(&job.session_id).is_some_and(|a| a.at == job.failed_at) {
                    state.asked.remove(&job.session_id);
                }
            }
            self.shared.pending.send_modify(|n| *n = n.saturating_sub(1));
        }
    }

    async fn decide(&self, job: &Job) {
        let deps = self.deps();
        let id = job.session_id.as_str();
        let mut probed: BTreeMap<String, bool> = BTreeMap::new();
        // every round asks for one more account, so this ends
        loop {
            let Some(session) = deps.session(id) else { return };
            let now = deps.now();
            let (asked, moves) = {
                let mut state = self.state();
                let Some(asked) = state.asked.get(id).copied().filter(|a| a.at == job.failed_at) else { return };
                state.moves.retain(|m| now - m.at < SWITCH_RULES.memory_ms);
                (asked, state.moves.clone())
            };
            let profiles = deps.profiles();
            let decision = decide_switch(&SwitchInput {
                enabled: deps.enabled(),
                now,
                session: session.clone(),
                failed_at: job.failed_at,
                was_busy: job.was_busy,
                typed: asked.typed,
                profiles: profiles.iter().map(|p| p.profile.clone()).collect(),
                usage: deps.usage(),
                probed: probed.clone(),
                moves,
            });
            let name = |id: &str| profiles.iter().find(|p| p.profile.id == id).map_or_else(|| "the account".to_owned(), |p| p.name.clone());
            match decision {
                SwitchDecision::Nothing { .. } => return,
                SwitchDecision::Probe { profile_id } => {
                    let fresh = deps.refresh_usage(&profile_id).await;
                    probed.insert(profile_id, fresh);
                }
                SwitchDecision::Stay { why, limit } => {
                    let source = name(&session.profile_id);
                    let why = match why {
                        StayWhy::Full => "and no other account has room",
                        StayWhy::Moved => "and this conversation was moved by itself not long ago",
                    };
                    deps.tell(id, NoticeLevel::Warn, &format!("{source} reached its {limit} {why}. The tab stays on {source}."));
                    return;
                }
                SwitchDecision::Move { to, limit } => {
                    let reached = format!("{} reached its {limit}", name(&session.profile_id));
                    self.move_to(id, &session, &to, &name(&to), &reached).await;
                    return;
                }
            }
        }
    }

    async fn move_to(&self, id: &str, session: &SwitchSession, to: &str, to_name: &str, reached: &str) {
        let deps = self.deps();
        let arrived = match deps.move_to(id, to).await {
            Ok(arrived) => arrived,
            Err(message) => {
                deps.tell(id, NoticeLevel::Warn, &format!("Could not continue on {to_name}: {message}"));
                return;
            }
        };
        let generation = {
            let mut state = self.state();
            state.moves.push(SwitchMove {
                conversation_id: session.conversation_id.clone().unwrap_or_default(),
                from: session.profile_id.clone(),
                to: to.to_owned(),
                at: deps.now(),
            });
            state.generation
        };
        deps.tell(&arrived, NoticeLevel::Info, &format!("Continued on {to_name}: {reached}."));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(PICK_UP_WATCH_MS);
        let me = self.clone();
        let to_name = to_name.to_owned();
        deps.spawn(Box::pin(async move {
            tokio::time::sleep_until(deadline).await;
            if me.state().generation != generation {
                return;
            }
            let deps = me.deps();
            // no message went out: a question of Claude Code stands before it, the trust in a new folder for one
            let waits = deps.session(&arrived).is_some_and(|there| there.moving != Some(true) && there.turn.is_none());
            if waits {
                deps.tell(&arrived, NoticeLevel::Warn, &format!("{to_name} waits for you in the tab: the task was not picked up yet."));
            }
        }));
    }
}

// ---------- the app ----------

/// [`SwitchDeps`] of the app. It finds the state through the handle when it is called.
pub struct AppSwitch(pub AppHandle);

impl SwitchDeps for AppSwitch {
    fn now(&self) -> i64 {
        now_ms()
    }

    fn enabled(&self) -> bool {
        let Some(state) = app_state(&self.0) else { return false };
        // the .cmd shim cannot hand Claude the message that makes it go on
        state.settings().auto_switch == Some(true) && state.claude_path().is_some_and(|claude| !crate::claude_binary::is_shim(&claude))
    }

    fn session(&self, session_id: &str) -> Option<SwitchSession> {
        let state = app_state(&self.0)?;
        let info = state.sessions.get(session_id).filter(|i| i.kind == SessionKind::Claude && i.status == SessionStatus::Running)?;
        Some(SwitchSession {
            profile_id: info.profile_id,
            model: info.model,
            conversation_id: info.claude_session_id,
            turn: info.turn,
            moving: info.moving.then_some(true),
            workflow: state.runs.watches(session_id),
        })
    }

    fn profiles(&self) -> Vec<NamedProfile> {
        let Some(state) = app_state(&self.0) else { return Vec::new() };
        state
            .profiles
            .list()
            .into_iter()
            .map(|p| NamedProfile {
                profile: SwitchProfile {
                    id: p.id,
                    auth: p.auth,
                    account: p.account.map(|a| LoginState { logged_in: a.logged_in }),
                },
                name: p.name,
            })
            .collect()
    }

    fn usage(&self) -> BTreeMap<String, UsageState> {
        app_state(&self.0).map(|s| s.usage.snapshot()).unwrap_or_default()
    }

    fn refresh_usage(&self, profile_id: &str) -> BoxFuture<'static, bool> {
        match app_state(&self.0) {
            Some(state) => Box::pin(state.usage.refresh(profile_id)),
            None => Box::pin(async { false }),
        }
    }

    fn move_to(&self, session_id: &str, to_profile_id: &str) -> BoxFuture<'static, Result<String, String>> {
        let (app, id, to) = (self.0.clone(), session_id.to_owned(), to_profile_id.to_owned());
        Box::pin(async move {
            let state = app_state(&app).ok_or("Multi Claude is shutting down.")?;
            let moved = crate::transfer::transfer_session(&state, &id, &to, true).await.map_err(|e| e.to_string())?;
            Ok(moved.id)
        })
    }

    fn tell(&self, session_id: &str, level: NoticeLevel, message: &str) {
        let Some(state) = app_state(&self.0) else { return };
        tell(&state, session_id, level, message);
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(task);
    }
}

/// With the window in front a line inside it is enough, the tab itself says the rest; otherwise a
/// notification whose click brings the tab to the front.
fn tell(state: &AppState, session_id: &str, level: NoticeLevel, message: &str) {
    if window::is_focused(&state.app) {
        notice(state, level, message);
    } else if state.settings().notifications != Some(false) {
        window::flash(&state.app);
        let title = state.sessions.get(session_id).map_or_else(|| "Multi Claude".to_owned(), |s| s.title);
        crate::app::notify::show_for(&state.app, &title, message, Some(session_id));
    }
}

/// What a turn change means for the auto switch (the end of `noteTurn`): typing makes a waiting
/// decision the user's; a turn that a usage limit cut off asks for one. `before` is the turn as
/// it was, `after` as it is now.
pub fn note_turn(state: &AppState, session_id: &str, event: &TurnEvent, before: Option<&Turn>, after: Option<&Turn>) {
    match event {
        TurnEvent::Typed => state.auto_switch.typed(session_id),
        TurnEvent::Failure { .. } if after.is_some_and(|t| matches!(t.kind, TurnKind::Failed { limit: true, .. })) => {
            state.auto_switch.cut(session_id, in_progress(before));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

    use mc_core::types::{StopError, UsageWindow};

    use super::*;

    // Date.UTC(2026, 9, 8, 12)
    const NOW: i64 = 1_791_460_800_000;
    const LATER: i64 = NOW + 3_600_000;
    const CUT_AT: i64 = NOW - 5000;

    fn cut_turn() -> Turn {
        failed_at(CUT_AT)
    }

    fn failed_at(at: i64) -> Turn {
        Turn { at, kind: TurnKind::Failed { error: StopError::RateLimit, text: None, limit: true, wait: None } }
    }

    fn window(used_pct: f64) -> Option<UsageWindow> {
        Some(UsageWindow { used_pct, resets_at: Some(LATER) })
    }

    fn full() -> UsageState {
        UsageState { five_hour: window(100.0), seven_day: window(40.0), ..UsageState::default() }
    }

    fn used(five_hour: f64) -> UsageState {
        UsageState { five_hour: window(five_hour), seven_day: window(10.0), ..UsageState::default() }
    }

    fn account(id: &str) -> NamedProfile {
        let mut name = id.to_owned();
        name[..1].make_ascii_uppercase();
        NamedProfile { profile: SwitchProfile { id: id.into(), auth: None, account: Some(LoginState { logged_in: true }) }, name }
    }

    fn tab(profile_id: &str, conversation_id: &str, turn: Option<Turn>) -> SwitchSession {
        SwitchSession { profile_id: profile_id.into(), model: None, conversation_id: Some(conversation_id.into()), turn, moving: None, workflow: false }
    }

    struct Fake {
        now: AtomicI64,
        enabled: AtomicBool,
        tabs: Mutex<HashMap<String, SwitchSession>>,
        /// What is known before anything is asked.
        usage: Mutex<BTreeMap<String, UsageState>>,
        /// What the accounts answer when they are asked; None for no answer.
        fresh: Mutex<HashMap<String, Option<UsageState>>>,
        move_fails: Option<String>,
        arrivals: AtomicUsize,
        probes: Mutex<Vec<String>>,
        moved: Mutex<Vec<String>>,
        told: Mutex<Vec<String>>,
    }

    impl SwitchDeps for Fake {
        fn now(&self) -> i64 {
            self.now.load(Ordering::SeqCst)
        }
        fn enabled(&self) -> bool {
            self.enabled.load(Ordering::SeqCst)
        }
        fn session(&self, session_id: &str) -> Option<SwitchSession> {
            lock(&self.tabs).get(session_id).cloned()
        }
        fn profiles(&self) -> Vec<NamedProfile> {
            vec![account("main"), account("elif"), account("emir")]
        }
        fn usage(&self) -> BTreeMap<String, UsageState> {
            lock(&self.usage).clone()
        }
        fn refresh_usage(&self, profile_id: &str) -> BoxFuture<'static, bool> {
            lock(&self.probes).push(profile_id.into());
            let answer = lock(&self.fresh).get(profile_id).cloned().flatten();
            let came = answer.is_some();
            if let Some(answer) = answer {
                lock(&self.usage).insert(profile_id.into(), answer);
            }
            Box::pin(async move { came })
        }
        fn move_to(&self, session_id: &str, to_profile_id: &str) -> BoxFuture<'static, Result<String, String>> {
            let outcome = match &self.move_fails {
                Some(message) => Err(message.clone()),
                None => {
                    lock(&self.moved).push(format!("{session_id}>{to_profile_id}"));
                    let next = format!("n{}", self.arrivals.fetch_add(1, Ordering::SeqCst) + 1);
                    let mut tabs = lock(&self.tabs);
                    let conversation = tabs.remove(session_id).and_then(|t| t.conversation_id);
                    tabs.insert(next.clone(), SwitchSession { conversation_id: conversation, ..tab(to_profile_id, "", None) });
                    Ok(next)
                }
            };
            Box::pin(async move { outcome })
        }
        fn tell(&self, session_id: &str, level: NoticeLevel, message: &str) {
            let level = match level {
                NoticeLevel::Info => "info",
                NoticeLevel::Warn => "warn",
                NoticeLevel::Error => "error",
            };
            lock(&self.told).push(format!("{session_id} {level}: {message}"));
        }
        fn spawn(&self, task: BoxFuture<'static, ()>) {
            tokio::spawn(task);
        }
    }

    #[derive(Default)]
    struct Setup {
        disabled: bool,
        usage: Vec<(&'static str, UsageState)>,
        fresh: Vec<(&'static str, Option<UsageState>)>,
        move_fails: Option<&'static str>,
    }

    fn setup(patch: Setup) -> (AutoSwitch, Arc<Fake>) {
        let mut usage: BTreeMap<String, UsageState> =
            [("main", used(50.0)), ("elif", used(30.0)), ("emir", used(60.0))].into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        usage.extend(patch.usage.into_iter().map(|(k, v)| (k.to_owned(), v)));
        let mut fresh: HashMap<String, Option<UsageState>> =
            [("main", Some(full())), ("elif", Some(used(30.0))), ("emir", Some(used(60.0)))].into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        fresh.extend(patch.fresh.into_iter().map(|(k, v)| (k.to_owned(), v)));
        let fake = Arc::new(Fake {
            now: AtomicI64::new(NOW),
            enabled: AtomicBool::new(!patch.disabled),
            tabs: Mutex::new(HashMap::from([("t1".to_owned(), tab("main", "c1", Some(cut_turn())))])),
            usage: Mutex::new(usage),
            fresh: Mutex::new(fresh),
            move_fails: patch.move_fails.map(str::to_owned),
            arrivals: AtomicUsize::new(0),
            probes: Mutex::default(),
            moved: Mutex::default(),
            told: Mutex::default(),
        });
        (AutoSwitch::new(fake.clone()), fake)
    }

    fn list(m: &Mutex<Vec<String>>) -> Vec<String> {
        lock(m).clone()
    }

    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn asks_both_accounts_for_fresh_numbers_moves_the_conversation_and_says_so() {
        let (auto, f) = setup(Setup::default());
        auto.cut("t1", true);
        auto.settled().await;
        assert_eq!(list(&f.probes), ["main", "elif"]);
        assert_eq!(list(&f.moved), ["t1>elif"]);
        assert_eq!(list(&f.told), ["n1 info: Continued on Elif: Main reached its 5-hour limit."]);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_touch_a_conversation_while_the_setting_is_off() {
        let (auto, f) = setup(Setup { disabled: true, ..Setup::default() });
        auto.cut("t1", true);
        auto.settled().await;
        assert!(list(&f.probes).is_empty());
        assert!(list(&f.moved).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn moves_nothing_when_the_fresh_numbers_do_not_show_a_full_limit() {
        let (auto, f) = setup(Setup { fresh: vec![("main", Some(used(70.0)))], ..Setup::default() });
        auto.cut("t1", true);
        auto.settled().await;
        assert_eq!(list(&f.probes), ["main"]);
        assert!(list(&f.moved).is_empty());
        assert!(list(&f.told).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn says_that_the_tab_stays_when_no_account_has_room() {
        let (auto, f) = setup(Setup { fresh: vec![("elif", Some(used(90.0))), ("emir", Some(used(100.0)))], ..Setup::default() });
        auto.cut("t1", true);
        auto.settled().await;
        assert_eq!(list(&f.probes), ["main", "elif", "emir"]);
        assert!(list(&f.moved).is_empty());
        assert_eq!(list(&f.told), ["t1 warn: Main reached its 5-hour limit and no other account has room. The tab stays on Main."]);
    }

    #[tokio::test(start_paused = true)]
    async fn drops_the_decision_when_the_user_types_into_the_tab_while_the_numbers_are_fetched() {
        let (auto, f) = setup(Setup::default());
        auto.cut("t1", true);
        auto.typed("t1");
        auto.settled().await;
        assert!(list(&f.moved).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn drops_the_decision_when_the_tab_has_gone_on_or_is_gone_by_the_time_the_numbers_are_in() {
        let (went, f) = setup(Setup::default());
        went.cut("t1", true);
        lock(&f.tabs).get_mut("t1").unwrap().turn = Some(Turn { at: NOW, kind: TurnKind::Working });
        went.settled().await;
        assert!(list(&f.moved).is_empty());

        let (gone, g) = setup(Setup::default());
        gone.cut("t1", true);
        lock(&g.tabs).remove("t1");
        gone.settled().await;
        assert!(list(&g.moved).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn decides_about_one_tab_at_a_time_and_gives_an_account_one_conversation_at_a_time() {
        let (auto, f) = setup(Setup::default());
        {
            let mut tabs = lock(&f.tabs);
            tabs.insert("t2".into(), tab("main", "c2", Some(cut_turn())));
            tabs.insert("t3".into(), tab("main", "c3", Some(cut_turn())));
        }
        auto.cut("t1", true);
        auto.cut("t2", true);
        auto.cut("t3", true);
        auto.settled().await;
        assert_eq!(list(&f.moved), ["t1>elif", "t2>emir"]);
        assert_eq!(list(&f.told)[2], "t3 warn: Main reached its 5-hour limit and no other account has room. The tab stays on Main.");
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_move_a_conversation_straight_on_from_where_it_just_arrived() {
        let (auto, f) = setup(Setup::default());
        auto.cut("t1", true);
        auto.settled().await;
        // the account it arrived on runs out too, five minutes later
        f.now.store(NOW + 5 * 60_000, Ordering::SeqCst);
        lock(&f.fresh).insert("elif".into(), Some(full()));
        lock(&f.tabs).get_mut("n1").unwrap().turn = Some(failed_at(NOW + 5 * 60_000 - 1000));
        auto.cut("n1", true);
        auto.settled().await;
        assert_eq!(list(&f.moved), ["t1>elif"]);
        assert_eq!(
            list(&f.told)[1],
            "n1 warn: Elif reached its 5-hour limit and this conversation was moved by itself not long ago. The tab stays on Elif."
        );
    }

    #[tokio::test(start_paused = true)]
    async fn says_when_the_move_itself_fails() {
        let (auto, f) = setup(Setup { move_fails: Some("\"Elif\" already has this conversation with a different continuation."), ..Setup::default() });
        auto.cut("t1", true);
        auto.settled().await;
        assert_eq!(
            list(&f.told),
            ["t1 warn: Could not continue on Elif: \"Elif\" already has this conversation with a different continuation."]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn says_when_the_task_was_not_picked_up_a_while_after_the_conversation_arrived() {
        let (auto, f) = setup(Setup::default());
        auto.cut("t1", true);
        auto.settled().await;
        advance(PICK_UP_WATCH_MS - 1).await;
        assert_eq!(list(&f.told).len(), 1);
        advance(1).await;
        assert_eq!(list(&f.told)[1], "n1 warn: Elif waits for you in the tab: the task was not picked up yet.");
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_quiet_about_a_conversation_that_did_pick_its_task_up_or_whose_tab_is_gone() {
        let (picked, f) = setup(Setup::default());
        picked.cut("t1", true);
        picked.settled().await;
        lock(&f.tabs).get_mut("n1").unwrap().turn = Some(Turn { at: NOW + 2000, kind: TurnKind::Working });
        advance(PICK_UP_WATCH_MS).await;
        assert_eq!(list(&f.told).len(), 1);

        let (closed, g) = setup(Setup::default());
        closed.cut("t1", true);
        closed.settled().await;
        lock(&g.tabs).remove("n1");
        advance(PICK_UP_WATCH_MS).await;
        assert_eq!(list(&g.told).len(), 1);

        let (stopped, h) = setup(Setup::default());
        stopped.cut("t1", true);
        stopped.settled().await;
        stopped.stop();
        advance(PICK_UP_WATCH_MS).await;
        assert_eq!(list(&h.told).len(), 1);
    }
}
