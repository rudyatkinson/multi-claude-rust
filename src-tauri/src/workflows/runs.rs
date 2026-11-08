//! The runs of workflows (`WorkflowRuns` of `src/main/workflows/runs.ts`): the I/O half around the
//! reducer of `mc_core::workflow::engine`. A run lives in memory only: it does not survive the app,
//! its tabs do.
//!
//! Every event goes through [`WorkflowRuns::dispatch`]: the reducer decides, the new run is kept,
//! changed outputs are saved, the window is told, and the effects are carried out. Opening a step's
//! session happens on a task of its own, never inside the event that asked for it (opening a
//! session reports back at once). Dispatches are made one at a time, so that the lists the window
//! gets are in the order the changes were made.
//!
//! What the runs need of the rest of the app comes in through [`RunsDeps`]: time, ids, the
//! launcher of a step, the sessions, the files and the window. Tests put a fake there and run on
//! a paused tokio clock.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mc_core::types::{HookReport, Place, StepState, Workflow, WorkflowRun};
use mc_core::workflow::{new_run, reduce, view_of, Effect, Notice, RunData, RunEvent, RunSession, KEEP_FINISHED, SILENT_MS};
use mc_core::error::{AppError, AppResult};

use crate::mcp_servers::BoxFuture;

/// What opening a step's session gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedStep {
    pub session_id: String,
    pub conversation_id: Option<String>,
}

/// True once the run that a step is being opened for has ended or moved on: nothing may be
/// opened then.
pub type Cancelled = Arc<dyn Fn() -> bool + Send + Sync>;

/// What the runs need of the rest of the app (`RunsDeps`).
pub trait RunsDeps: Send + Sync + 'static {
    /// `Date.now()`.
    fn now(&self) -> i64;
    /// A new run's id.
    fn new_id(&self) -> String;
    /// One spelling per folder, to tell whether two runs share a folder.
    fn folder_key(&self, cwd: &str) -> String;
    /// Opens the session of a step and sends it its message. The error is the text the step
    /// pauses with.
    fn start_step(&self, run: RunData, index: usize, cancelled: Cancelled) -> BoxFuture<'static, Result<StartedStep, String>>;
    fn end_session(&self, session_id: &str);
    /// What the conversation in a step's tab last said; None when it said nothing yet.
    fn last_answer(&self, run: RunData, index: usize) -> BoxFuture<'static, Option<String>>;
    /// Keeps a step's output beside its prompt. Failing is harmless.
    fn save_output(&self, run: &RunData, index: usize, text: &str);
    fn notify(&self, run: &WorkflowRun, notice: Notice, index: usize);
    /// `runs:changed`.
    fn changed(&self, runs: Vec<WorkflowRun>);
    /// Runs a task in the background (the app's async runtime; the test's in tests).
    fn spawn(&self, task: BoxFuture<'static, ()>);
}

#[derive(Default)]
struct Inner {
    runs: Vec<RunData>,
    /// Where the first tab of a run was asked to appear (`runPlaces` of `index.ts`).
    places: HashMap<String, Option<Place>>,
    /// The one watchdog of a run: its token. A watchdog that fires with another token, or after
    /// its run lost it, does nothing.
    watchdogs: HashMap<String, u64>,
    next_token: u64,
}

impl Inner {
    fn get(&self, id: &str) -> Option<&RunData> {
        self.runs.iter().find(|r| r.id == id)
    }

    fn list(&self) -> Vec<WorkflowRun> {
        self.runs.iter().map(view_of).collect()
    }

    /// Lets the oldest finished runs go.
    fn trim(&mut self) {
        let mut finished: Vec<(i64, String)> =
            self.runs.iter().filter(|r| !r.is_active()).map(|r| (r.ended_at.unwrap_or(0), r.id.clone())).collect();
        if finished.len() <= KEEP_FINISHED {
            return;
        }
        // stable, as `Array.prototype.sort` is
        finished.sort_by_key(|(ended_at, _)| *ended_at);
        let drop: HashSet<String> = finished.drain(..finished.len() - KEEP_FINISHED).map(|(_, id)| id).collect();
        self.runs.retain(|r| !drop.contains(&r.id));
        self.forget_gone();
    }

    /// What was kept for runs that are no longer on the list.
    fn forget_gone(&mut self) {
        let ids: HashSet<&str> = self.runs.iter().map(|r| r.id.as_str()).collect();
        self.places.retain(|id, _| ids.contains(id.as_str()));
        self.watchdogs.retain(|id, _| ids.contains(id.as_str()));
    }
}

struct Shared {
    deps: Arc<dyn RunsDeps>,
    state: Mutex<Inner>,
    /// One dispatch at a time (see the module comment). Never held while waiting for anything.
    dispatching: Mutex<()>,
}

/// The runs of workflows. Cloning gives another handle to the same runs.
#[derive(Clone)]
pub struct WorkflowRuns {
    shared: Arc<Shared>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `!text.trim()` of JavaScript: nothing but white space (and byte order marks).
fn blank(text: &str) -> bool {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}').is_empty()
}

impl WorkflowRuns {
    pub fn new(deps: Arc<dyn RunsDeps>) -> Self {
        WorkflowRuns { shared: Arc::new(Shared { deps, state: Mutex::default(), dispatching: Mutex::new(()) }) }
    }

    fn state(&self) -> MutexGuard<'_, Inner> {
        lock(&self.shared.state)
    }

    fn deps(&self) -> &dyn RunsDeps {
        &*self.shared.deps
    }

    /// The runs as the window sees them.
    pub fn list(&self) -> Vec<WorkflowRun> {
        self.state().list()
    }

    /// The runs that still have something to do or to wait for.
    pub fn active(&self) -> Vec<WorkflowRun> {
        self.state().runs.iter().filter(|r| r.is_active()).map(view_of).collect()
    }

    /// Begins a run of `workflow` in `cwd`; its first step is opened right after. `place` is where
    /// the window asked for the first tab of the run. Refused while another run is active in the
    /// same folder: the two would edit the same files without knowing of each other.
    ///
    /// The place is taken here, not set after the call as the TypeScript did: the first step may
    /// be opened on another thread before this returns.
    pub fn start(&self, workflow: &Workflow, cwd: &str, task: &str, place: Option<Place>) -> AppResult<WorkflowRun> {
        let deps = self.deps();
        let key = deps.folder_key(cwd);
        let id = {
            let mut state = self.state();
            if let Some(busy) = state.runs.iter().find(|r| r.is_active() && deps.folder_key(&r.cwd) == key) {
                return Err(AppError::refused(format!(
                    "\"{}\" is already running in this folder. Stop it or wait until it is finished.",
                    busy.name
                )));
            }
            let run = new_run(workflow, deps.new_id(), cwd, task, deps.now());
            let id = run.id.clone();
            state.places.insert(id.clone(), place);
            state.runs.push(run);
            id
        };
        self.dispatch(&id, RunEvent::Begin);
        let state = self.state();
        state.get(&id).map(view_of).ok_or_else(|| AppError::refused("The run was stopped."))
    }

    pub fn approve(&self, id: &str) {
        self.dispatch(id, RunEvent::Approve);
    }

    pub fn retry(&self, id: &str) {
        self.dispatch(id, RunEvent::Retry);
    }

    pub fn stop(&self, id: &str) {
        self.dispatch(id, RunEvent::Stop);
    }

    /// Stops what is still running, without telling anyone: the app is on its way out.
    pub fn stop_all(&self) {
        let _one_at_a_time = lock(&self.shared.dispatching);
        let mut state = self.state();
        state.watchdogs.clear();
        state.runs.clear();
        state.places.clear();
    }

    /// Takes a run that is over off the list.
    pub fn dismiss(&self, id: &str) {
        let _one_at_a_time = lock(&self.shared.dispatching);
        let list = {
            let mut state = self.state();
            match state.get(id) {
                Some(run) if !run.is_active() => {}
                _ => return,
            }
            state.runs.retain(|r| r.id != id);
            state.forget_gone();
            state.list()
        };
        self.deps().changed(list);
    }

    /// Takes what the current step's tab last said as its output, for a step that is finished and
    /// did not say so: the answer that was held back, else the last one in its conversation. False
    /// when there is nothing to take, or the run moved on while the conversation was read.
    pub async fn take_answer(&self, id: &str) -> bool {
        let (run, index, held) = {
            let state = self.state();
            let Some(run) = state.get(id) else { return false };
            let Some(step) = run.steps.get(run.current) else { return false };
            if !matches!(step.state, StepState::Running | StepState::Paused) {
                return false;
            }
            (run.clone(), run.current, step.held.clone())
        };
        let text = match held {
            Some(held) => Some(held),
            None => self.deps().last_answer(run, index).await,
        };
        let Some(text) = text.filter(|t| !blank(t)) else { return false };
        // the run may have moved while the file was read
        if self.state().get(id).map(|r| r.current) != Some(index) {
            return false;
        }
        self.dispatch(id, RunEvent::TakeAnswer { text: Some(text) });
        true
    }

    /// What a step of a run gave, while the run is on the list.
    pub fn output(&self, id: &str, index: usize) -> Option<String> {
        self.state().get(id)?.steps.get(index)?.output.clone()
    }

    /// Where the window asked for the first tab of a run.
    pub fn first_place(&self, id: &str) -> Option<Place> {
        self.state().places.get(id).cloned().flatten()
    }

    /// True for the tab of a step that a run is working on or waiting at.
    pub fn watches(&self, session_id: &str) -> bool {
        self.state().runs.iter().any(|r| r.is_active() && current_tab(r) == Some(session_id))
    }

    /// What a tab's hooks report goes to every active run whose current step is in that tab.
    pub fn hook(&self, session_id: &str, report: &HookReport) {
        let ids: Vec<String> = {
            let state = self.state();
            state.runs.iter().filter(|r| r.is_active() && current_tab(r) == Some(session_id)).map(|r| r.id.clone()).collect()
        };
        for id in ids {
            self.dispatch(&id, RunEvent::hook(session_id, report.clone()));
        }
    }

    /// The list of sessions changed.
    pub fn sessions_changed(&self, list: Vec<RunSession>) {
        let ids: Vec<String> = self.state().runs.iter().filter(|r| r.is_active()).map(|r| r.id.clone()).collect();
        for id in ids {
            self.dispatch(&id, RunEvent::Sessions { list: list.clone() });
        }
    }

    /// The user typed into a tab.
    pub fn input(&self, session_id: &str) {
        let ids: Vec<String> = {
            let state = self.state();
            state
                .runs
                .iter()
                .filter(|r| r.is_active() && r.steps.get(r.current).is_some_and(|s| s.needs_you) && current_tab(r) == Some(session_id))
                .map(|r| r.id.clone())
                .collect()
        };
        for id in ids {
            self.dispatch(&id, RunEvent::Input { session_id: session_id.to_owned() });
        }
    }

    /// Hands an event to the reducer and does what comes of it. The effects are carried out once
    /// the next dispatch may go ahead: telling the user asks the window whether it is in front,
    /// which waits for the window's thread, and that thread may itself be waiting to dispatch.
    fn dispatch(&self, id: &str, event: RunEvent) {
        let effects = self.apply(id, &event);
        for effect in effects {
            self.perform(id, effect);
        }
    }

    /// The part of a dispatch that is made one at a time: the reducer, the new run, the saved
    /// outputs and the list for the window. Gives the effects to carry out.
    fn apply(&self, id: &str, event: &RunEvent) -> Vec<Effect> {
        let _one_at_a_time = lock(&self.shared.dispatching);
        let deps = self.deps();
        let (after, saves, list, effects) = {
            let mut state = self.state();
            let Some(pos) = state.runs.iter().position(|r| r.id == id) else { return Vec::new() };
            let reduced = reduce(&state.runs[pos], event, deps.now());
            if !reduced.changed() && reduced.effects.is_empty() {
                return Vec::new();
            }
            let effects = reduced.effects;
            let run = reduced.run.into_owned();
            let before = &state.runs[pos];
            let saves: Vec<(usize, String)> = run
                .steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| {
                    let output = step.output.as_ref()?;
                    (before.steps.get(index).and_then(|b| b.output.as_ref()) != Some(output)).then(|| (index, output.clone()))
                })
                .collect();
            let moved = run.current != before.current;
            if !run.is_active() || moved {
                state.watchdogs.remove(id);
            }
            let after = (!saves.is_empty()).then(|| run.clone());
            state.runs[pos] = run;
            state.trim();
            (after, saves, state.list(), effects)
        };
        if let Some(run) = &after {
            for (index, text) in &saves {
                deps.save_output(run, *index, text);
            }
        }
        deps.changed(list);
        effects
    }

    fn perform(&self, id: &str, effect: Effect) {
        match effect {
            Effect::Start { index } => {
                let runs = self.clone();
                let id = id.to_owned();
                // not from inside the event that asked for it: opening a session reports back at once
                self.deps().spawn(Box::pin(async move {
                    let Some(run) = runs.wanted(&id, index) else { return };
                    let cancelled: Cancelled = {
                        let runs = runs.clone();
                        let id = id.clone();
                        Arc::new(move || runs.wanted(&id, index).is_none())
                    };
                    let outcome = runs.deps().start_step(run, index, cancelled).await;
                    let event = match outcome {
                        Ok(started) => RunEvent::Started { index, session_id: started.session_id, conversation_id: started.conversation_id },
                        Err(message) => RunEvent::StartFailed { index, message },
                    };
                    runs.dispatch(&id, event);
                }));
            }
            Effect::End { session_id } => self.deps().end_session(&session_id),
            Effect::Watch { index, session_id } => {
                let token = {
                    let mut state = self.state();
                    state.next_token += 1;
                    let token = state.next_token;
                    state.watchdogs.insert(id.to_owned(), token);
                    token
                };
                // the deadline is taken now, not when the task first runs
                let deadline = tokio::time::Instant::now() + Duration::from_millis(SILENT_MS);
                let runs = self.clone();
                let id = id.to_owned();
                self.deps().spawn(Box::pin(async move {
                    tokio::time::sleep_until(deadline).await;
                    let mine = {
                        let mut state = runs.state();
                        let mine = state.watchdogs.get(&id) == Some(&token);
                        if mine {
                            state.watchdogs.remove(&id);
                        }
                        mine
                    };
                    if mine {
                        runs.dispatch(&id, RunEvent::Silent { index, session_id });
                    }
                }));
            }
            Effect::Notify { notice, index } => {
                let view = self.state().get(id).map(view_of);
                if let Some(view) = view {
                    self.deps().notify(&view, notice, index);
                }
            }
        }
    }

    /// The run, while it still wants the step at `index` opened.
    fn wanted(&self, id: &str, index: usize) -> Option<RunData> {
        let state = self.state();
        let run = state.get(id)?;
        let wanted = run.is_active() && run.current == index && run.steps.get(index).is_some_and(|s| s.state == StepState::Starting);
        wanted.then(|| run.clone())
    }
}

/// The tab of the step a run is at.
fn current_tab(run: &RunData) -> Option<&str> {
    run.steps.get(run.current)?.session_id.as_deref()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

    use mc_core::types::{Extra, HookSignal, RunState, SessionStatus, WorkflowStep};
    use tokio::sync::oneshot;

    use super::*;

    fn step(id: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            name: id.into(),
            profile_id: "main".into(),
            model: None,
            effort: None,
            permission_mode: None,
            continues: None,
            prompt: format!("do {id}"),
            gate: true,
            extra: Extra::new(),
        }
    }

    fn flow(steps: Vec<WorkflowStep>) -> Workflow {
        Workflow { id: "w1".into(), name: "Flow".into(), steps, extra: Extra::new() }
    }

    fn three() -> Workflow {
        let mut build = step("build");
        build.profile_id = "work".into();
        let mut review = step("review");
        review.continues = Some("plan".into());
        flow(vec![step("plan"), build, review])
    }

    type StartFn = dyn Fn(RunData, usize, Cancelled) -> BoxFuture<'static, Result<StartedStep, String>> + Send + Sync;

    #[derive(Default)]
    struct Log {
        starts: Vec<(RunData, usize)>,
        ended: Vec<String>,
        saved: Vec<String>,
        notices: Vec<String>,
        changes: usize,
    }

    struct Fake {
        n: AtomicI64,
        start: Box<StartFn>,
        log: Mutex<Log>,
        answers: Mutex<HashMap<usize, String>>,
    }

    impl RunsDeps for Fake {
        fn now(&self) -> i64 {
            1000 + self.n.load(Ordering::SeqCst)
        }
        fn new_id(&self) -> String {
            format!("run{}", self.n.fetch_add(1, Ordering::SeqCst) + 1)
        }
        fn folder_key(&self, cwd: &str) -> String {
            cwd.to_lowercase()
        }
        fn start_step(&self, run: RunData, index: usize, cancelled: Cancelled) -> BoxFuture<'static, Result<StartedStep, String>> {
            lock(&self.log).starts.push((run.clone(), index));
            (self.start)(run, index, cancelled)
        }
        fn end_session(&self, session_id: &str) {
            lock(&self.log).ended.push(session_id.into());
        }
        fn last_answer(&self, _run: RunData, index: usize) -> BoxFuture<'static, Option<String>> {
            let answer = lock(&self.answers).get(&index).cloned();
            Box::pin(async move { answer })
        }
        fn save_output(&self, _run: &RunData, index: usize, text: &str) {
            lock(&self.log).saved.push(format!("{index}:{text}"));
        }
        fn notify(&self, run: &WorkflowRun, notice: Notice, index: usize) {
            let kind = match notice {
                Notice::Gate => "gate",
                Notice::NeedsYou => "needsYou",
                Notice::Paused => "paused",
                Notice::Done => "done",
            };
            lock(&self.log).notices.push(format!("{kind}:{}", run.steps[index].name));
        }
        fn changed(&self, _runs: Vec<WorkflowRun>) {
            lock(&self.log).changes += 1;
        }
        fn spawn(&self, task: BoxFuture<'static, ()>) {
            tokio::spawn(task);
        }
    }

    fn fake_with<F>(start: F) -> (WorkflowRuns, Arc<Fake>)
    where
        F: Fn(RunData, usize, Cancelled) -> BoxFuture<'static, Result<StartedStep, String>> + Send + Sync + 'static,
    {
        let fake = Arc::new(Fake { n: AtomicI64::new(0), start: Box::new(start), log: Mutex::default(), answers: Mutex::default() });
        (WorkflowRuns::new(fake.clone()), fake)
    }

    /// Every step opens in a tab of its own at once: `t1`, `t2`, ... with conversations `c1`, `c2`, ...
    fn fake() -> (WorkflowRuns, Arc<Fake>) {
        fake_with(|_run, index, _cancelled| {
            Box::pin(async move { Ok(StartedStep { session_id: format!("t{}", index + 1), conversation_id: Some(format!("c{}", index + 1)) }) })
        })
    }

    /// Lets the spawned tasks run.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn report(signal: HookSignal) -> HookReport {
        HookReport { conversation_id: None, signal }
    }

    fn answer(runs: &WorkflowRuns, tab: &str, text: &str) {
        runs.hook(tab, &report(HookSignal::Prompt));
        runs.hook(tab, &report(HookSignal::Stop { text: text.into(), background: false }));
    }

    fn notices(fake: &Fake) -> Vec<String> {
        lock(&fake.log).notices.clone()
    }

    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn runs_a_workflow_from_the_first_step_to_the_last() {
        let (runs, f) = fake();
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        assert_eq!(run.state, RunState::Running);
        assert!(lock(&f.log).starts.is_empty());
        settle().await;
        assert_eq!(lock(&f.log).starts.len(), 1);
        assert!(runs.watches("t1"));

        answer(&runs, "t1", "the plan");
        assert_eq!(runs.list()[0].state, RunState::Waiting);
        assert_eq!(notices(&f), ["gate:plan"]);
        assert_eq!(lock(&f.log).saved, ["0:the plan"]);
        assert_eq!(runs.output(&run.id, 0).as_deref(), Some("the plan"));

        runs.approve(&run.id);
        settle().await;
        // the second step is opened with the first one's output at hand
        {
            let log = lock(&f.log);
            let (seen, index) = &log.starts[1];
            assert_eq!(*index, 1);
            assert_eq!(seen.steps[0].output.as_deref(), Some("the plan"));
        }
        assert!(!runs.watches("t1"));
        assert!(runs.watches("t2"));

        answer(&runs, "t2", "built");
        runs.approve(&run.id);
        settle().await;
        answer(&runs, "t3", "fine");
        assert_eq!(runs.list()[0].state, RunState::Done);
        assert_eq!(notices(&f), ["gate:plan", "gate:build", "done:review"]);
        assert!(runs.active().is_empty());
        assert!(lock(&f.log).changes > 0);
    }

    #[tokio::test(start_paused = true)]
    async fn refuses_a_second_run_in_the_same_folder_however_it_is_spelled() {
        let (runs, _f) = fake();
        let first = runs.start(&three(), "C:\\App", "one", None).unwrap();
        let refused = runs.start(&three(), "c:\\app", "two", None).unwrap_err();
        assert!(refused.to_string().contains("already running in this folder"), "{refused}");
        assert_eq!(runs.start(&three(), "C:\\Other", "two", None).unwrap().state, RunState::Running);
        runs.stop(&first.id);
        assert_eq!(runs.start(&three(), "c:\\app", "three", None).unwrap().state, RunState::Running);
    }

    #[tokio::test(start_paused = true)]
    async fn pauses_when_a_step_cannot_be_opened_and_tries_again_on_retry() {
        let fail = Arc::new(AtomicBool::new(true));
        let failing = fail.clone();
        let (runs, f) = fake_with(move |_run, _index, _cancelled| {
            let fails = failing.load(Ordering::SeqCst);
            Box::pin(async move {
                if fails {
                    Err("The account of step \"plan\" no longer exists.".to_string())
                } else {
                    Ok(StartedStep { session_id: "t1".into(), conversation_id: None })
                }
            })
        });
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        assert_eq!(runs.list()[0].state, RunState::Paused);
        assert_eq!(runs.list()[0].steps[0].problem.as_deref(), Some("The account of step \"plan\" no longer exists."));
        assert_eq!(notices(&f), ["paused:plan"]);

        fail.store(false, Ordering::SeqCst);
        runs.retry(&run.id);
        settle().await;
        let first = &runs.list()[0].steps[0];
        assert_eq!((first.state, first.session_id.as_deref()), (StepState::Running, Some("t1")));
    }

    #[tokio::test(start_paused = true)]
    async fn opens_nothing_for_a_run_that_was_stopped_first_and_tells_a_start_under_way() {
        let (runs, f) = fake();
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        runs.stop(&run.id);
        settle().await;
        assert!(lock(&f.log).starts.is_empty());

        let seen: Arc<Mutex<Option<Cancelled>>> = Arc::default();
        let (open, opened) = oneshot::channel::<StartedStep>();
        let opened = Mutex::new(Some(opened));
        let keep = seen.clone();
        let (slow_runs, _g) = fake_with(move |_run, _index, cancelled| {
            *lock(&keep) = Some(cancelled);
            let opened = lock(&opened).take().expect("opened once");
            Box::pin(async move { opened.await.map_err(|e| e.to_string()) })
        });
        let slow = slow_runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        let cancelled = lock(&seen).clone().expect("the step was being opened");
        assert!(!cancelled());
        slow_runs.stop(&slow.id);
        assert!(cancelled());
        open.send(StartedStep { session_id: "t1".into(), conversation_id: None }).unwrap();
        settle().await;
        assert_eq!(slow_runs.list()[0].state, RunState::Stopped);
        assert_eq!(slow_runs.list()[0].steps[0].session_id, None);
    }

    #[tokio::test(start_paused = true)]
    async fn says_that_a_step_needs_the_user_when_its_message_is_not_sent_in_time() {
        let (runs, f) = fake();
        runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        advance(SILENT_MS - 1).await;
        assert!(notices(&f).is_empty());
        advance(1).await;
        assert_eq!(notices(&f), ["needsYou:plan"]);
        assert_eq!(runs.list()[0].steps[0].needs_you, Some(true));
        runs.input("t1");
        assert_eq!(runs.list()[0].steps[0].needs_you, None);
    }

    #[tokio::test(start_paused = true)]
    async fn stays_quiet_when_the_message_was_sent_in_time() {
        let (runs, f) = fake();
        runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        runs.hook("t1", &report(HookSignal::Prompt));
        advance(SILENT_MS * 2).await;
        assert!(notices(&f).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn takes_the_answer_that_was_held_back_else_the_last_one_in_the_tab() {
        let (runs, f) = fake();
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        assert!(!runs.take_answer(&run.id).await);

        lock(&f.answers).insert(0, "read from the conversation".into());
        runs.hook("t1", &report(HookSignal::Prompt));
        runs.hook("t1", &report(HookSignal::Stop { text: "held back".into(), background: true }));
        assert!(runs.take_answer(&run.id).await);
        assert_eq!(runs.output(&run.id, 0).as_deref(), Some("held back"));

        runs.approve(&run.id);
        settle().await;
        lock(&f.answers).insert(1, "read from the conversation".into());
        assert!(runs.take_answer(&run.id).await);
        assert_eq!(runs.output(&run.id, 1).as_deref(), Some("read from the conversation"));
        // at a gate there is nothing to take
        assert!(!runs.take_answer(&run.id).await);
    }

    #[tokio::test(start_paused = true)]
    async fn takes_no_answer_that_is_only_white_space() {
        let (runs, f) = fake();
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        lock(&f.answers).insert(0, " \n\t".into());
        assert!(!runs.take_answer(&run.id).await);
        assert_eq!(runs.output(&run.id, 0), None);
    }

    #[tokio::test(start_paused = true)]
    async fn ends_the_process_of_an_approved_plan_and_follows_the_tabs() {
        let (runs, f) = fake();
        let run = runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        runs.hook("t1", &report(HookSignal::Prompt));
        runs.hook("t1", &HookReport { conversation_id: Some("c-new".into()), signal: HookSignal::Plan { text: "PLAN".into() } });
        runs.approve(&run.id);
        assert_eq!(lock(&f.log).ended, ["t1"]);
        settle().await;
        runs.sessions_changed(vec![RunSession { id: "t2".into(), profile_id: "work".into(), status: SessionStatus::Running, replaces: None }]);
        assert_eq!(runs.list()[0].steps[0].session_id, None);
        assert_eq!(runs.list()[0].state, RunState::Running);
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_the_last_few_finished_runs_and_lets_them_be_dismissed() {
        let (runs, _f) = fake();
        let mut ids = Vec::new();
        for i in 0..7 {
            let run = runs.start(&flow(vec![step("only")]), &format!("C:\\App{i}"), "task", None).unwrap();
            ids.push(run.id.clone());
            runs.stop(&run.id);
        }
        let listed = |runs: &WorkflowRuns| runs.list().into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(listed(&runs), ids[2..]);
        runs.dismiss(&ids[6]);
        assert_eq!(listed(&runs), ids[2..6]);
        // a run that is under way stays
        let live = runs.start(&three(), "C:\\Live", "task", None).unwrap();
        runs.dismiss(&live.id);
        assert!(listed(&runs).contains(&live.id));
    }

    #[tokio::test(start_paused = true)]
    async fn forgets_everything_when_the_app_goes_down() {
        let (runs, f) = fake();
        runs.start(&three(), "C:\\App", "task", None).unwrap();
        settle().await;
        runs.stop_all();
        assert!(runs.list().is_empty());
        advance(SILENT_MS * 2).await;
        assert!(notices(&f).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn remembers_where_the_first_tab_of_a_run_goes_while_the_run_is_listed() {
        let (runs, _f) = fake();
        let place = Place { group_id: Some("g2".into()), after_id: None };
        let run = runs.start(&flow(vec![step("only")]), "C:\\App", "task", Some(place.clone())).unwrap();
        assert_eq!(runs.first_place(&run.id), Some(place));
        runs.stop(&run.id);
        runs.dismiss(&run.id);
        assert_eq!(runs.first_place(&run.id), None);
    }
}
