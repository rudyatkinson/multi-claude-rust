//! How a run moves from step to step (`src/main/workflows/engine.ts`). Everything here is a pure
//! function of the run and of what happened; what has to be done about it comes back as
//! [`Effect`]s for the I/O half to carry out.
//!
//! # "Unchanged"
//!
//! [`reduce`] borrows the run and gives back a [`Cow`]: `Cow::Borrowed` (the very same run) where
//! the TypeScript returned the same object, `Cow::Owned` wherever it built a new one, even when
//! the new one happens to hold the same values. [`Reduced::changed`] tells the two apart; a
//! result that is unchanged has no effects.
//!
//! # What the I/O half does with it (`WorkflowRuns` of `src/main/workflows/runs.ts`)
//!
//! The runs live in memory only: a run does not survive the app, its tabs do. The holder of the
//! runs needs these dependencies (`RunsDeps`):
//!
//! - `now() -> i64` and `new_id() -> String` (a run's id);
//! - `folder_key(cwd) -> String`: one spelling per folder. A run is refused while another active
//!   run has the same key, with `"<name>" is already running in this folder. Stop it or wait
//!   until it is finished.`;
//! - `start_step(run, index, cancelled) -> Result<StartedStep { session_id, conversation_id? }>`:
//!   opens the session of a step and sends it [`launch_message`](super::launch_message) (see
//!   `startStep` of `launcher.ts`); `cancelled()` turns true when the run ended or moved on
//!   meanwhile, and nothing may be opened then (`The run was stopped.`);
//! - `end_session(session_id)`;
//! - `last_answer(run, index) -> Option<String>`: what the step's conversation last said
//!   ([`read_last_answer`](super::read_last_answer) on its transcript);
//! - `save_output(run, index, text)`: [`write_step_output`](super::write_step_output), failing
//!   silently;
//! - `notify(view, notice, index)` and `changed(list of views)`.
//!
//! It dispatches every event to [`reduce`] and then:
//!
//! - does nothing more when the result is unchanged and has no effects;
//! - otherwise keeps the new run, calls `save_output` for every step whose `output` differs from
//!   before, clears the run's watchdog when the run is no longer active or `current` moved, lets
//!   the oldest finished runs go (by `ended_at`) beyond [`KEEP_FINISHED`], calls `changed`, and
//!   performs the effects in order:
//!   - on [`Effect::Start`], not from inside the event that asked for it (opening a session reports
//!     back at once); only while the run is active, at that index, and the step is `starting`.
//!     The outcome is dispatched as [`RunEvent::Started`] or [`RunEvent::StartFailed`] (with the
//!     error's text);
//!   - on [`Effect::End`], `end_session`;
//!   - on [`Effect::Watch`], (re)arms the run's one watchdog: after [`SILENT_MS`] it dispatches
//!     [`RunEvent::Silent`] with the same index and session;
//!   - on [`Effect::Notify`], `notify` with [`view_of`] the run as it is now.
//!
//! Events come from: `begin` on start; `approve`, `retry`, `stop` from the window; `hook` for
//! every active run whose current step is in that tab; `sessions` for every active run when the
//! session list changes; `input` for a run whose current step needs the user in that tab;
//! `takeAnswer` after taking `held`, else `last_answer`, and only when it is not blank and the
//! run is still at the same step. `stop_all` on quit drops every run and timer without telling
//! anyone; `dismiss` takes a finished run off the list.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use super::schema::thread_of;
use crate::types::{
    HookReport, HookSignal, RunState, RunStep, SessionInfo, SessionStatus, StepState, Workflow, WorkflowRun, WorkflowStep,
};

/// A step whose message is not sent after this long is probably held up by a dialog in its tab.
pub const SILENT_MS: u64 = 20_000;
/// Runs that are over are kept for looking at, this many of them.
pub const KEEP_FINISHED: usize = 5;

pub const RESTARTED: &str = "Its tab was restarted. Go on in the tab, or retry the step.";
pub const CLOSED: &str = "Its tab was closed. Retry the step to run it again.";
pub const ENDED: &str = "Claude ended in its tab. Go on there after a restart, or retry the step.";

/// A conversation of a run. Steps that continue an earlier step share that step's conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thread {
    /// The tab it lives in, while that tab exists.
    pub session_id: Option<String>,
    /// Claude Code's id of it.
    pub conversation_id: Option<String>,
    /// The account whose config dir holds its latest state.
    pub profile_id: Option<String>,
}

/// A step of a run (`StepRun` in the TypeScript).
#[derive(Debug, Clone, PartialEq)]
pub struct StepData {
    /// The step as it was designed when the run began.
    pub def: WorkflowStep,
    pub state: StepState,
    /// Index of the step that started this step's conversation; its own when it starts one.
    pub thread: usize,
    pub session_id: Option<String>,
    /// Its message has been sent, so an answer that ends from now on is the answer to it.
    pub armed: bool,
    pub needs_you: bool,
    pub output: Option<String>,
    /// An answer after which background work was still running.
    pub held: Option<String>,
    /// It ended on a plan that still waits for approval in its tab.
    pub plan_open: bool,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunData {
    pub id: String,
    pub workflow_id: String,
    pub name: String,
    pub cwd: String,
    pub task: String,
    pub state: RunState,
    pub current: usize,
    pub steps: Vec<StepData>,
    /// By the index of the step that started the conversation.
    pub threads: BTreeMap<usize, Thread>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

impl RunData {
    /// `isActive`: the run still has something to do or to wait for.
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }
}

/// A session as the engine needs to know it (`SessionRef` of `engine.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSession {
    pub id: String,
    pub profile_id: String,
    pub status: SessionStatus,
    /// The session whose tab this one took over.
    pub replaces: Option<String>,
}

impl From<&SessionInfo> for RunSession {
    fn from(info: &SessionInfo) -> Self {
        RunSession { id: info.id.clone(), profile_id: info.profile_id.clone(), status: info.status, replaces: info.replaces.clone() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    Begin,
    /// The step's session is open.
    Started { index: usize, session_id: String, conversation_id: Option<String> },
    StartFailed { index: usize, message: String },
    Hook { session_id: String, conversation_id: Option<String>, signal: HookSignal },
    Sessions { list: Vec<RunSession> },
    /// The user typed into a tab.
    Input { session_id: String },
    /// The step's message has not been sent a while after its session opened.
    Silent { index: usize, session_id: String },
    /// The user lets the run pass a gate.
    Approve,
    Retry,
    /// The user takes what the tab's conversation last said as the step's output; None when
    /// there is nothing.
    TakeAnswer { text: Option<String> },
    Stop,
}

impl RunEvent {
    /// The event of a hook report from the tab `session_id`.
    pub fn hook(session_id: impl Into<String>, report: HookReport) -> RunEvent {
        RunEvent::Hook { session_id: session_id.into(), conversation_id: report.conversation_id, signal: report.signal }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    Gate,
    NeedsYou,
    Paused,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Open the session of this step.
    Start { index: usize },
    /// End a process whose plan dialog must not be answered any more.
    End { session_id: String },
    /// Tell if the step's message is not sent within the usual time.
    Watch { index: usize, session_id: String },
    Notify { notice: Notice, index: usize },
}

#[derive(Debug)]
pub struct Reduced<'a> {
    /// `Cow::Borrowed` when the run is the same as before.
    pub run: Cow<'a, RunData>,
    pub effects: Vec<Effect>,
}

impl Reduced<'_> {
    /// False when the run came back as it was (`run === before` in the TypeScript).
    pub fn changed(&self) -> bool {
        matches!(self.run, Cow::Owned(_))
    }
}

/// A run of `workflow`, before its first step is opened.
pub fn new_run(workflow: &Workflow, id: impl Into<String>, cwd: impl Into<String>, task: impl Into<String>, now: i64) -> RunData {
    RunData {
        id: id.into(),
        workflow_id: workflow.id.clone(),
        name: workflow.name.clone(),
        cwd: cwd.into(),
        task: task.into(),
        state: RunState::Running,
        current: 0,
        steps: workflow
            .steps
            .iter()
            .enumerate()
            .map(|(index, def)| StepData {
                def: def.clone(),
                state: StepState::Pending,
                thread: thread_of(&workflow.steps, index),
                session_id: None,
                armed: false,
                needs_you: false,
                output: None,
                held: None,
                plan_open: false,
                problem: None,
            })
            .collect(),
        threads: BTreeMap::new(),
        started_at: now,
        ended_at: None,
    }
}

/// The run as the window is told about it: without prompts and outputs.
pub fn view_of(run: &RunData) -> WorkflowRun {
    WorkflowRun {
        id: run.id.clone(),
        workflow_id: run.workflow_id.clone(),
        name: run.name.clone(),
        cwd: run.cwd.clone(),
        state: run.state,
        current: u32::try_from(run.current).unwrap_or(u32::MAX),
        steps: run
            .steps
            .iter()
            .map(|s| RunStep {
                id: s.def.id.clone(),
                name: s.def.name.clone(),
                profile_id: s.def.profile_id.clone(),
                state: s.state,
                session_id: s.session_id.clone(),
                needs_you: s.needs_you.then_some(true),
                background: s.held.is_some().then_some(true),
                has_output: s.output.is_some(),
                problem: s.problem.clone(),
            })
            .collect(),
        started_at: run.started_at,
        ended_at: run.ended_at,
    }
}

/// A string that JavaScript takes for true: present and not empty.
fn truthy(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

/// `pause`: the step at `index` cannot go on by itself.
fn pause(mut base: RunData, index: usize, problem: &str) -> RunData {
    if let Some(step) = base.steps.get_mut(index) {
        step.state = StepState::Paused;
        step.problem = Some(problem.to_string());
        step.needs_you = false;
    }
    base.state = RunState::Paused;
    base
}

/// Leaves the step at `index` behind and opens the next one.
fn advance(mut base: RunData, index: usize, effects: &mut Vec<Effect>) -> RunData {
    if let Some(done) = base.steps.get_mut(index) {
        if done.plan_open {
            if let Some(session_id) = truthy(&done.session_id) {
                effects.push(Effect::End { session_id: session_id.to_string() });
            }
        }
        done.state = StepState::Done;
        done.plan_open = false;
        done.needs_you = false;
        done.problem = None;
        done.held = None;
    }
    if let Some(next) = base.steps.get_mut(index + 1) {
        next.state = StepState::Starting;
    }
    base.current = index + 1;
    base.state = RunState::Running;
    effects.push(Effect::Start { index: index + 1 });
    base
}

/// The step at `index` has its output.
fn complete(mut base: RunData, index: usize, output: &str, plan_open: bool, now: i64, effects: &mut Vec<Effect>) -> RunData {
    let last = index + 1 == base.steps.len();
    let Some(step) = base.steps.get_mut(index) else { return base };
    step.output = Some(output.to_string());
    step.plan_open = plan_open;
    step.held = None;
    step.needs_you = false;
    step.problem = None;
    if last {
        effects.push(Effect::Notify { notice: Notice::Done, index });
        step.state = StepState::Done;
        base.state = RunState::Done;
        base.ended_at = Some(now);
        return base;
    }
    if step.def.gate {
        effects.push(Effect::Notify { notice: Notice::Gate, index });
        step.state = StepState::Gate;
        base.state = RunState::Waiting;
        return base;
    }
    advance(base, index, effects)
}

/// What `event` makes of `run` at `now`, and what has to be done about it.
///
/// Where the TypeScript would throw (an event other than `begin` or `stop` for a run without
/// steps), the run is left as it is.
pub fn reduce<'a>(run: &'a RunData, event: &RunEvent, now: i64) -> Reduced<'a> {
    let mut effects = Vec::new();
    let same = || Reduced { run: Cow::Borrowed(run), effects: Vec::new() };
    let index = run.current;

    // before the check for a run that is over, as in the TypeScript
    if let RunEvent::Begin = event {
        if run.steps.get(index).is_none_or(|step| step.state != StepState::Pending) {
            return same();
        }
        effects.push(Effect::Start { index: 0 });
        let mut next = run.clone();
        next.steps[0].state = StepState::Starting;
        return Reduced { run: Cow::Owned(next), effects };
    }

    // what is over stays as it is; its tabs are ordinary tabs from then on
    if !run.is_active() {
        return same();
    }

    let Some(step) = run.steps.get(index) else {
        if let RunEvent::Stop = event {
            let mut next = run.clone();
            next.state = RunState::Stopped;
            next.ended_at = Some(now);
            return Reduced { run: Cow::Owned(next), effects };
        }
        return same();
    };
    let owned = |next: RunData, effects: Vec<Effect>| Reduced { run: Cow::Owned(next), effects };

    match event {
        RunEvent::Begin => unreachable!("handled above"),

        RunEvent::Started { index: at, session_id, conversation_id } => {
            if *at != index || step.state != StepState::Starting {
                return same();
            }
            let mut next = run.clone();
            let s = &mut next.steps[index];
            s.state = StepState::Running;
            s.session_id = Some(session_id.clone());
            s.armed = false;
            s.needs_you = false;
            s.problem = None;
            s.held = None;
            s.plan_open = false;
            let conversation_id =
                conversation_id.clone().or_else(|| run.threads.get(&step.thread).and_then(|t| t.conversation_id.clone()));
            let thread = next.threads.entry(step.thread).or_default();
            thread.session_id = Some(session_id.clone());
            thread.conversation_id = conversation_id;
            thread.profile_id = Some(step.def.profile_id.clone());
            // the earlier steps of the same conversation are found in this tab from now on
            for earlier in next.steps[..index].iter_mut().filter(|s| s.thread == step.thread) {
                earlier.session_id = Some(session_id.clone());
                earlier.plan_open = false;
            }
            effects.push(Effect::Watch { index, session_id: session_id.clone() });
            next.state = RunState::Running;
            owned(next, effects)
        }

        RunEvent::StartFailed { index: at, message } => {
            if *at != index || step.state != StepState::Starting {
                return same();
            }
            effects.push(Effect::Notify { notice: Notice::Paused, index });
            owned(pause(run.clone(), index, message), effects)
        }

        RunEvent::Hook { session_id, conversation_id, signal } => {
            if step.session_id.as_deref() != Some(session_id.as_str()) {
                return same();
            }
            let live = matches!(step.state, StepState::Running | StepState::Paused);
            if !live && step.state != StepState::Gate {
                return same();
            }
            // the conversation may have been replaced inside the tab (/clear, /resume)
            let base: Cow<'a, RunData> = match truthy(conversation_id) {
                Some(conversation) => {
                    let mut next = run.clone();
                    next.threads.entry(step.thread).or_default().conversation_id = Some(conversation.to_string());
                    Cow::Owned(next)
                }
                None => Cow::Borrowed(run),
            };

            match signal {
                HookSignal::Prompt => {
                    if !live {
                        return Reduced { run: base, effects };
                    }
                    // a message went out: the step is on its way, also after it had paused
                    let mut next = base.into_owned();
                    let s = &mut next.steps[index];
                    s.state = StepState::Running;
                    s.armed = true;
                    s.needs_you = false;
                    s.problem = None;
                    next.state = RunState::Running;
                    owned(next, effects)
                }
                HookSignal::Plan { text } => {
                    if step.state == StepState::Gate {
                        let mut next = base.into_owned();
                        next.steps[index].output = Some(text.clone());
                        next.steps[index].plan_open = true;
                        return owned(next, effects);
                    }
                    let next = complete(base.into_owned(), index, text, true, now, &mut effects);
                    owned(next, effects)
                }
                HookSignal::Stop { text, background } => {
                    if !step.armed {
                        return same();
                    }
                    let mut next = base.into_owned();
                    // the user went on with the conversation at the gate: its latest answer is what goes on
                    if step.state == StepState::Gate {
                        next.steps[index].output = Some(text.clone());
                        next.steps[index].plan_open = false;
                        return owned(next, effects);
                    }
                    if *background {
                        next.steps[index].held = Some(text.clone());
                        next.steps[index].needs_you = false;
                        return owned(next, effects);
                    }
                    let next = complete(next, index, text, false, now, &mut effects);
                    owned(next, effects)
                }
                HookSignal::Failure { reason, .. } => {
                    if step.state != StepState::Running {
                        return same();
                    }
                    effects.push(Effect::Notify { notice: Notice::Paused, index });
                    owned(pause(base.into_owned(), index, reason), effects)
                }
                // Claude Code's own wait for a limit: when it goes on, that shows as a message being sent
                HookSignal::LimitWait { .. } => same(),
                HookSignal::NeedsYou { .. } => {
                    if !live || step.needs_you {
                        return same();
                    }
                    effects.push(Effect::Notify { notice: Notice::NeedsYou, index });
                    let mut next = base.into_owned();
                    next.steps[index].needs_you = true;
                    owned(next, effects)
                }
            }
        }

        RunEvent::Sessions { list } => {
            let by_id: HashMap<&str, &RunSession> = list.iter().map(|s| (s.id.as_str(), s)).collect();
            let successor: HashMap<&str, &RunSession> =
                list.iter().filter_map(|s| truthy(&s.replaces).map(|replaced| (replaced, s))).collect();
            let mut next: Option<RunData> = None;
            for (key, thread) in &run.threads {
                let Some(tab) = truthy(&thread.session_id) else { continue };
                if let Some(heir) = successor.get(tab) {
                    // a restart, or a move to another account, which then holds the conversation
                    let entry = next.get_or_insert_with(|| run.clone()).threads.entry(*key).or_default();
                    entry.session_id = Some(heir.id.clone());
                    entry.profile_id = Some(heir.profile_id.clone());
                } else if !by_id.contains_key(tab) {
                    next.get_or_insert_with(|| run.clone()).threads.entry(*key).or_default().session_id = None;
                }
            }
            if let Some(next) = next.as_mut() {
                for s in &mut next.steps {
                    let Some(tab) = truthy(&s.session_id) else { continue };
                    if let Some(heir) = successor.get(tab) {
                        s.session_id = Some(heir.id.clone());
                        s.armed = false;
                        s.needs_you = false;
                        s.plan_open = false;
                    } else if !by_id.contains_key(tab) {
                        s.session_id = None;
                        s.needs_you = false;
                        s.plan_open = false;
                    }
                }
            }
            // while a step is being started the engine itself replaces and ends sessions
            let watched = truthy(&step.session_id);
            if step.state != StepState::Running || watched.is_none() {
                return match next {
                    Some(next) => owned(next, effects),
                    None => same(),
                };
            }
            let tab = next.as_ref().unwrap_or(run).steps[index].session_id.as_deref();
            let problem = match tab {
                None => Some(CLOSED),
                Some(tab) if Some(tab) != step.session_id.as_deref() => Some(RESTARTED),
                Some(tab) if by_id.get(tab).is_some_and(|s| s.status == SessionStatus::Exited) => Some(ENDED),
                Some(_) => None,
            };
            match (problem, next) {
                (Some(problem), next) => owned(pause(next.unwrap_or_else(|| run.clone()), index, problem), effects),
                (None, Some(next)) => owned(next, effects),
                (None, None) => same(),
            }
        }

        RunEvent::Input { session_id } => {
            if step.session_id.as_deref() == Some(session_id.as_str()) && step.needs_you {
                let mut next = run.clone();
                next.steps[index].needs_you = false;
                return owned(next, effects);
            }
            same()
        }

        RunEvent::Silent { index: at, session_id } => {
            if *at != index
                || step.session_id.as_deref() != Some(session_id.as_str())
                || step.state != StepState::Running
                || step.armed
                || step.needs_you
            {
                return same();
            }
            effects.push(Effect::Notify { notice: Notice::NeedsYou, index });
            let mut next = run.clone();
            next.steps[index].needs_you = true;
            owned(next, effects)
        }

        RunEvent::Approve => {
            if step.state != StepState::Gate {
                return same();
            }
            let next = advance(run.clone(), index, &mut effects);
            owned(next, effects)
        }

        RunEvent::Retry => {
            if step.state != StepState::Paused {
                return same();
            }
            effects.push(Effect::Start { index });
            let mut next = run.clone();
            let s = &mut next.steps[index];
            s.state = StepState::Starting;
            s.problem = None;
            s.armed = false;
            s.needs_you = false;
            s.held = None;
            s.plan_open = false;
            next.state = RunState::Running;
            owned(next, effects)
        }

        RunEvent::TakeAnswer { text } => {
            let Some(text) = text else { return same() };
            if !matches!(step.state, StepState::Running | StepState::Paused) {
                return same();
            }
            let next = complete(run.clone(), index, text, false, now, &mut effects);
            owned(next, effects)
        }

        RunEvent::Stop => {
            let mut next = run.clone();
            next.steps[index].needs_you = false;
            next.state = RunState::Stopped;
            next.ended_at = Some(now);
            owned(next, effects)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::types::{AskWhat, Extra, LimitWaitStage, StopError};

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
    fn with(mut s: WorkflowStep, patch: impl FnOnce(&mut WorkflowStep)) -> WorkflowStep {
        patch(&mut s);
        s
    }
    fn ungated(id: &str) -> WorkflowStep {
        with(step(id), |s| s.gate = false)
    }
    fn flow(steps: Vec<WorkflowStep>) -> Workflow {
        Workflow { id: "w1".into(), name: "Flow".into(), steps, extra: Extra::new() }
    }
    fn three() -> Workflow {
        flow(vec![
            step("plan"),
            with(step("build"), |s| s.profile_id = "work".into()),
            with(step("review"), |s| s.continues = Some("plan".into())),
        ])
    }

    fn begun_with(workflow: &Workflow) -> RunData {
        new_run(workflow, "r1", "C:\\code\\app", "the task", 100)
    }
    fn begun() -> RunData {
        begun_with(&three())
    }

    struct Played {
        run: RunData,
        effects: Vec<Effect>,
    }

    /// Applies events in order; gives the run and every effect that came up.
    fn play(run: &RunData, events: &[RunEvent]) -> Played {
        let mut run = run.clone();
        let mut effects = Vec::new();
        for event in events {
            let Reduced { run: next, effects: more } = reduce(&run, event, 200);
            effects.extend(more);
            run = next.into_owned();
        }
        Played { run, effects }
    }

    /// The run comes back as it was from every one of the events (`toBe(run)`).
    fn same(run: &RunData, events: &[RunEvent]) -> bool {
        events.iter().all(|event| !reduce(run, event, 200).changed())
    }

    fn hook(session_id: &str, signal: HookSignal) -> RunEvent {
        RunEvent::Hook { session_id: session_id.into(), conversation_id: None, signal }
    }
    fn prompt(session_id: &str) -> RunEvent {
        hook(session_id, HookSignal::Prompt)
    }
    fn stop(session_id: &str, text: &str) -> RunEvent {
        hook(session_id, HookSignal::Stop { text: text.into(), background: false })
    }
    fn stop_in_background(session_id: &str, text: &str) -> RunEvent {
        hook(session_id, HookSignal::Stop { text: text.into(), background: true })
    }
    fn plan(session_id: &str, text: &str) -> RunEvent {
        hook(session_id, HookSignal::Plan { text: text.into() })
    }
    fn failure(session_id: &str, reason: &str) -> RunEvent {
        hook(session_id, HookSignal::Failure { reason: reason.into(), error: StopError::RateLimit, text: String::new() })
    }
    fn needs_you(session_id: &str) -> RunEvent {
        hook(session_id, HookSignal::NeedsYou { what: AskWhat::Permission })
    }
    fn started_in(index: usize, session_id: &str, conversation_id: &str) -> RunEvent {
        RunEvent::Started { index, session_id: session_id.into(), conversation_id: Some(conversation_id.into()) }
    }
    fn started(index: usize, session_id: &str) -> RunEvent {
        started_in(index, session_id, &format!("conv-{session_id}"))
    }
    fn session(id: &str, profile_id: &str, status: SessionStatus, replaces: Option<&str>) -> RunSession {
        RunSession { id: id.into(), profile_id: profile_id.into(), status, replaces: replaces.map(String::from) }
    }
    fn tab(id: &str) -> RunSession {
        session(id, "main", SessionStatus::Running, None)
    }
    fn tabs(list: Vec<RunSession>) -> RunEvent {
        RunEvent::Sessions { list }
    }
    fn silent(index: usize, session_id: &str) -> RunEvent {
        RunEvent::Silent { index, session_id: session_id.into() }
    }

    fn name_of(value: impl Serialize) -> String {
        serde_json::to_value(value).unwrap().as_str().unwrap().to_string()
    }
    fn states(run: &RunData) -> String {
        let steps: Vec<String> = run.steps.iter().map(|s| name_of(s.state)).collect();
        format!("{}: {}", name_of(run.state), steps.join(" "))
    }
    fn notices(effects: &[Effect]) -> usize {
        effects.iter().filter(|e| matches!(e, Effect::Notify { .. })).count()
    }

    /// The first step is open and its message is sent.
    fn working_with(workflow: &Workflow) -> RunData {
        play(&begun_with(workflow), &[RunEvent::Begin, started(0, "t1"), prompt("t1")]).run
    }
    fn working() -> RunData {
        working_with(&three())
    }

    mod a_run_step_by_step {
        use super::*;

        #[test]
        fn opens_the_first_step_when_it_begins() {
            let p = play(&begun(), &[RunEvent::Begin]);
            assert_eq!(states(&p.run), "running: starting pending pending");
            assert_eq!(p.effects, [Effect::Start { index: 0 }]);
            // only once
            assert!(play(&p.run, &[RunEvent::Begin]).effects.is_empty());
        }

        #[test]
        fn notes_the_tab_and_the_conversation_of_a_step_that_has_started_and_watches_for_its_message() {
            let p = play(&begun(), &[RunEvent::Begin, started_in(0, "t1", "c1")]);
            assert_eq!(states(&p.run), "running: running pending pending");
            assert_eq!(p.run.steps[0].session_id.as_deref(), Some("t1"));
            assert_eq!(
                p.run.threads[&0],
                Thread { session_id: Some("t1".into()), conversation_id: Some("c1".into()), profile_id: Some("main".into()) }
            );
            assert!(p.effects.contains(&Effect::Watch { index: 0, session_id: "t1".into() }));
        }

        #[test]
        fn waits_at_the_gate_with_the_answer_and_goes_on_when_it_is_approved() {
            let gate = play(&working(), &[stop("t1", "the plan")]);
            assert_eq!(states(&gate.run), "waiting: gate pending pending");
            assert_eq!(gate.run.steps[0].output.as_deref(), Some("the plan"));
            assert_eq!(gate.effects, [Effect::Notify { notice: Notice::Gate, index: 0 }]);

            let next = play(&gate.run, &[RunEvent::Approve]);
            assert_eq!(states(&next.run), "running: done starting pending");
            assert_eq!(next.run.current, 1);
            assert_eq!(next.effects, [Effect::Start { index: 1 }]);
        }

        #[test]
        fn goes_straight_on_from_a_step_without_a_gate() {
            let p = play(&working_with(&flow(vec![ungated("a"), step("b")])), &[stop("t1", "out")]);
            assert_eq!(states(&p.run), "running: done starting");
            assert_eq!(p.effects, [Effect::Start { index: 1 }]);
        }

        #[test]
        fn is_done_after_its_last_step_gate_or_not() {
            let p = play(&working_with(&flow(vec![step("only")])), &[stop("t1", "out")]);
            assert_eq!(states(&p.run), "done: done");
            assert_eq!(p.run.ended_at, Some(200));
            assert_eq!(p.effects, [Effect::Notify { notice: Notice::Done, index: 0 }]);
        }

        #[test]
        fn does_not_take_an_answer_for_the_step_before_its_own_message_was_sent() {
            let idle = play(&begun(), &[RunEvent::Begin, started(0, "t1")]);
            assert!(same(&idle.run, &[stop("t1", "left over from before")]));
        }

        #[test]
        fn ignores_what_other_tabs_report() {
            assert!(same(&working(), &[stop("elsewhere", "x"), prompt("elsewhere"), failure("elsewhere", "x")]));
        }

        #[test]
        fn keeps_the_latest_answer_while_the_user_goes_on_talking_at_the_gate() {
            let p = play(&working(), &[stop("t1", "first"), prompt("t1"), stop("t1", "second, after a question")]);
            assert_eq!(states(&p.run), "waiting: gate pending pending");
            assert_eq!(p.run.steps[0].output.as_deref(), Some("second, after a question"));
            // told once, when the gate was reached
            assert_eq!(notices(&p.effects), 1);
        }

        #[test]
        fn walks_through_all_three_steps() {
            let p = play(
                &working(),
                &[stop("t1", "plan"), RunEvent::Approve, started(1, "t2"), prompt("t2"), stop("t2", "built"), RunEvent::Approve],
            );
            assert_eq!(states(&p.run), "running: done done starting");
            let p = play(&p.run, &[started_in(2, "t3", "conv-t1"), prompt("t3"), stop("t3", "looks good")]);
            assert_eq!(states(&p.run), "done: done done done");
            assert_eq!(
                p.run.steps.iter().map(|s| s.output.as_deref()).collect::<Vec<_>>(),
                [Some("plan"), Some("built"), Some("looks good")]
            );
        }
    }

    mod a_plan_that_waits_for_approval_in_its_tab {
        use super::*;

        #[test]
        fn is_the_output_of_the_step_and_its_process_is_ended_when_the_run_moves_on() {
            let gate = play(&working(), &[plan("t1", "PLAN")]);
            assert_eq!(states(&gate.run), "waiting: gate pending pending");
            assert_eq!(gate.run.steps[0].output.as_deref(), Some("PLAN"));
            assert!(gate.run.steps[0].plan_open);

            let next = play(&gate.run, &[RunEvent::Approve]);
            assert_eq!(next.effects, [Effect::End { session_id: "t1".into() }, Effect::Start { index: 1 }]);
            assert!(!next.run.steps[0].plan_open);
        }

        #[test]
        fn is_told_once_although_two_events_carry_it() {
            let p = play(&working(), &[plan("t1", "PLAN"), plan("t1", "PLAN")]);
            assert_eq!(p.run.steps[0].output.as_deref(), Some("PLAN"));
            assert_eq!(notices(&p.effects), 1);
        }

        #[test]
        fn is_replaced_by_a_revised_plan() {
            let p = play(&working(), &[plan("t1", "v1"), plan("t1", "v2")]);
            assert_eq!(p.run.steps[0].output.as_deref(), Some("v2"));
            assert!(p.run.steps[0].plan_open);
        }

        #[test]
        fn needs_no_ending_once_the_user_approved_it_in_the_tab_and_claude_went_on() {
            let p = play(&working(), &[plan("t1", "PLAN"), stop("t1", "implemented it myself")]);
            let first = &p.run.steps[0];
            assert_eq!(first.output.as_deref(), Some("implemented it myself"));
            assert!(!first.plan_open);
            assert_eq!(first.state, StepState::Gate);
            assert_eq!(play(&p.run, &[RunEvent::Approve]).effects, [Effect::Start { index: 1 }]);
        }

        #[test]
        fn ends_the_process_at_once_in_a_step_without_a_gate() {
            let p = play(&working_with(&flow(vec![ungated("a"), step("b")])), &[plan("t1", "PLAN")]);
            assert_eq!(p.effects, [Effect::End { session_id: "t1".into() }, Effect::Start { index: 1 }]);
        }
    }

    mod a_step_that_cannot_go_on_by_itself {
        use super::*;

        #[test]
        fn pauses_on_an_api_error_and_runs_again_on_retry() {
            let failed = play(&working(), &[failure("t1", "The account reached a usage limit.")]);
            assert_eq!(states(&failed.run), "paused: paused pending pending");
            assert_eq!(failed.run.steps[0].problem.as_deref(), Some("The account reached a usage limit."));
            assert_eq!(failed.effects, [Effect::Notify { notice: Notice::Paused, index: 0 }]);

            let again = play(&failed.run, &[RunEvent::Retry]);
            assert_eq!(states(&again.run), "running: starting pending pending");
            assert_eq!(again.run.steps[0].problem, None);
            assert_eq!(again.effects, [Effect::Start { index: 0 }]);
        }

        #[test]
        fn goes_on_when_the_user_gives_it_a_hand_in_its_tab() {
            let p = play(&working(), &[failure("t1", "x"), prompt("t1")]);
            assert_eq!(states(&p.run), "running: running pending pending");
            assert_eq!(states(&play(&p.run, &[stop("t1", "done after all")]).run), "waiting: gate pending pending");
        }

        #[test]
        fn pauses_when_its_session_cannot_be_opened() {
            let p = play(&begun(), &[RunEvent::Begin, RunEvent::StartFailed { index: 0, message: "Folder not found".into() }]);
            assert_eq!(states(&p.run), "paused: paused pending pending");
            assert_eq!(p.run.steps[0].problem.as_deref(), Some("Folder not found"));
            assert!(p.effects.contains(&Effect::Notify { notice: Notice::Paused, index: 0 }));
        }

        #[test]
        fn retries_only_what_has_paused() {
            let run = working();
            assert!(same(&run, &[RunEvent::Retry]));
            assert!(same(&run, &[RunEvent::Approve]));
        }

        #[test]
        fn pauses_when_its_tab_is_closed_ends_or_is_restarted() {
            let closed = play(&working(), &[tabs(vec![tab("other")])]);
            assert_eq!(states(&closed.run), "paused: paused pending pending");
            assert_eq!(closed.run.steps[0].session_id, None);
            assert!(closed.run.steps[0].problem.as_deref().unwrap().contains("closed"));

            let ended = play(&working(), &[tabs(vec![session("t1", "main", SessionStatus::Exited, None)])]);
            assert!(ended.run.steps[0].problem.as_deref().unwrap().contains("ended"));
            assert_eq!(ended.run.steps[0].session_id.as_deref(), Some("t1"));

            let restarted = play(&working(), &[tabs(vec![session("t9", "main", SessionStatus::Running, Some("t1"))])]);
            let first = &restarted.run.steps[0];
            assert_eq!(first.state, StepState::Paused);
            assert_eq!(first.session_id.as_deref(), Some("t9"));
            assert!(!first.armed);
            assert!(first.problem.as_deref().unwrap().contains("restarted"));
            assert_eq!(restarted.run.threads[&0].session_id.as_deref(), Some("t9"));
        }

        #[test]
        fn finishes_in_the_restarted_tab_once_the_user_sends_a_message_there() {
            let p = play(
                &working(),
                &[
                    tabs(vec![session("t9", "main", SessionStatus::Running, Some("t1"))]),
                    stop("t9", "not ours"),
                    prompt("t9"),
                    stop("t9", "ours"),
                ],
            );
            assert_eq!(states(&p.run), "waiting: gate pending pending");
            assert_eq!(p.run.steps[0].output.as_deref(), Some("ours"));
        }

        #[test]
        fn follows_a_conversation_that_was_moved_to_another_account() {
            let p = play(&working(), &[tabs(vec![session("t9", "work", SessionStatus::Running, Some("t1"))])]);
            let thread = &p.run.threads[&0];
            assert_eq!(thread.session_id.as_deref(), Some("t9"));
            assert_eq!(thread.profile_id.as_deref(), Some("work"));
            assert_eq!(thread.conversation_id.as_deref(), Some("conv-t1"));
        }

        #[test]
        fn leaves_the_tabs_alone_while_the_engine_itself_opens_a_step() {
            // the plan step waits at its gate; its tab ends and is replaced as the review step takes it over
            let p = play(
                &working(),
                &[stop("t1", "plan"), RunEvent::Approve, started(1, "t2"), prompt("t2"), stop("t2", "built"), RunEvent::Approve],
            );
            let p = play(
                &p.run,
                &[
                    tabs(vec![session("t1", "main", SessionStatus::Exited, None), tab("t2")]),
                    tabs(vec![session("t3", "main", SessionStatus::Running, Some("t1")), tab("t2")]),
                ],
            );
            assert_eq!(states(&p.run), "running: done done starting");
            assert_eq!(p.run.threads[&0].session_id.as_deref(), Some("t3"));
            let p = play(&p.run, &[started_in(2, "t3", "conv-t1")]);
            assert_eq!(states(&p.run), "running: done done running");
            // the plan step is found in the tab that carries its conversation on
            assert_eq!(
                p.run.steps.iter().map(|s| s.session_id.as_deref()).collect::<Vec<_>>(),
                [Some("t3"), Some("t2"), Some("t3")]
            );
        }

        #[test]
        fn forgets_a_tab_of_an_earlier_step_that_was_closed() {
            let p = play(&working(), &[stop("t1", "plan"), RunEvent::Approve, started(1, "t2"), tabs(vec![tab("t2")])]);
            assert_eq!(p.run.threads[&0].session_id, None);
            assert_eq!(p.run.steps[0].session_id, None);
            assert_eq!(states(&p.run), "running: done running pending");
        }
    }

    mod a_step_that_waits_for_the_user {
        use super::*;

        #[test]
        fn says_so_once_until_the_user_types_there() {
            let asked = play(&working(), &[needs_you("t1"), needs_you("t1")]);
            assert!(asked.run.steps[0].needs_you);
            assert_eq!(asked.effects, [Effect::Notify { notice: Notice::NeedsYou, index: 0 }]);
            assert!(same(&asked.run, &[RunEvent::Input { session_id: "elsewhere".into() }]));
            assert!(!play(&asked.run, &[RunEvent::Input { session_id: "t1".into() }]).run.steps[0].needs_you);
        }

        #[test]
        fn says_so_when_its_message_is_not_sent_in_time_as_with_a_dialog_in_the_way() {
            let idle = play(&begun(), &[RunEvent::Begin, started(0, "t1")]).run;
            let p = play(&idle, &[silent(0, "t1")]);
            assert!(p.run.steps[0].needs_you);
            assert_eq!(p.effects, [Effect::Notify { notice: Notice::NeedsYou, index: 0 }]);
            // the message went out after all
            assert!(!play(&p.run, &[prompt("t1")]).run.steps[0].needs_you);
        }

        #[test]
        fn does_not_say_so_for_a_step_whose_message_was_sent_or_that_has_moved_on() {
            let run = working();
            assert!(same(&run, &[silent(0, "t1")]));
            assert!(same(&run, &[silent(0, "old-tab")]));
            let gate = play(&run, &[stop("t1", "out")]).run;
            assert!(same(&gate, &[needs_you("t1")]));
        }

        #[test]
        fn holds_an_answer_back_while_background_work_is_running_and_takes_the_one_after_it() {
            let held = play(&working(), &[stop_in_background("t1", "started the build")]);
            assert_eq!(states(&held.run), "running: running pending pending");
            assert_eq!(held.run.steps[0].held.as_deref(), Some("started the build"));
            assert_eq!(held.run.steps[0].output, None);
            assert_eq!(view_of(&held.run).steps[0].background, Some(true));

            let finished = play(&held.run, &[stop("t1", "build is green")]);
            assert_eq!(finished.run.steps[0].output.as_deref(), Some("build is green"));
            assert_eq!(finished.run.steps[0].state, StepState::Gate);
            assert_eq!(finished.run.steps[0].held, None);
        }

        #[test]
        fn takes_an_answer_the_user_points_at() {
            let run = working();
            assert!(same(&run, &[RunEvent::TakeAnswer { text: None }]));
            let taken = play(&run, &[RunEvent::TakeAnswer { text: Some("from the tab".into()) }]).run;
            assert_eq!(taken.steps[0].output.as_deref(), Some("from the tab"));
            assert_eq!(taken.steps[0].state, StepState::Gate);
        }

        #[test]
        fn is_not_taken_to_wait_for_the_user_while_claude_code_waits_for_a_usage_limit_by_itself() {
            let run = working();
            for stage in [LimitWaitStage::Resumed, LimitWaitStage::NeedsEnter, LimitWaitStage::Ended] {
                let waited = reduce(&run, &hook("t1", HookSignal::LimitWait { stage }), 200);
                assert!(!waited.changed());
                assert!(waited.effects.is_empty());
            }
        }
    }

    mod a_run_that_is_over {
        use super::*;

        #[test]
        fn stops_where_it_is_and_reacts_to_nothing_afterwards() {
            let p = play(&working(), &[RunEvent::Stop]);
            assert_eq!(p.run.state, RunState::Stopped);
            assert_eq!(p.run.ended_at, Some(200));
            assert!(same(&p.run, &[stop("t1", "late"), RunEvent::Approve, RunEvent::Retry, tabs(vec![])]));
        }

        #[test]
        fn ignores_a_session_that_opens_after_it_was_stopped() {
            let p = play(&begun(), &[RunEvent::Begin, RunEvent::Stop]);
            assert!(same(&p.run, &[started(0, "t1")]));
        }

        #[test]
        fn shows_the_window_its_steps_without_their_texts() {
            let view = view_of(&play(&working(), &[stop("t1", "secret plan")]).run);
            assert_eq!(
                (view.id.as_str(), view.name.as_str(), view.cwd.as_str(), view.state, view.current),
                ("r1", "Flow", "C:\\code\\app", RunState::Waiting, 0)
            );
            assert_eq!(
                view.steps[0],
                RunStep {
                    id: "plan".into(),
                    name: "plan".into(),
                    profile_id: "main".into(),
                    state: StepState::Gate,
                    session_id: Some("t1".into()),
                    needs_you: None,
                    background: None,
                    has_output: true,
                    problem: None,
                }
            );
            let json = serde_json::to_string(&view).unwrap();
            assert!(!json.contains("secret plan"));
            assert!(!json.contains("the task"));
        }
    }
}
