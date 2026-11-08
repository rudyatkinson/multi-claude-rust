//! Opening the session of a workflow step (`startStep` of `src/main/workflows/launcher.ts`). The
//! pure parts (file names, the first message, prompt values, the prompt file) are in
//! `mc_core::workflow::launcher`.
//!
//! A step with a conversation of its own gets a new tab (or, when it is tried again, its old one).
//! A step that goes on with an earlier conversation takes over that conversation's tab; when its
//! account is another one, the conversation is copied there first.

use std::path::PathBuf;

use mc_core::error::{AppError, AppResult};
use mc_core::transfer::{can_replace, copy_conversation, find_transcript, plan_transfer};
use mc_core::types::{LaunchOptions, Place, Profile, ResumeConversation, SessionInfo, SessionKind, SessionRequest, SessionResume};
use mc_core::workflow::{launch_message, prompt_values, render_prompt, run_folder, write_step_prompt, RunData};

use super::runs::StartedStep;
use crate::mcp_servers::BoxFuture;

/// A process that is still writing when it is told to end gets this long.
pub const END_BEFORE_RESUME_MS: u64 = 3000;

/// What opening a step needs of the rest of the app (`LauncherDeps`).
pub trait LauncherDeps: Send + Sync {
    fn profile(&self, id: &str) -> Option<Profile>;
    /// `effectiveConfigDir`: the account's CLAUDE_CONFIG_DIR, or `~/.claude` for Main.
    fn config_dir(&self, profile: &Profile) -> PathBuf;
    /// The folder that holds one folder per run, with the prompts and outputs of its steps.
    fn runs_dir(&self) -> PathBuf;
    /// Where the first tab of a run was asked to appear.
    fn first_place(&self, run_id: &str) -> Option<Place>;
    /// A new tab, announced at `place`.
    fn create(&self, request: &SessionRequest, place: Option<Place>) -> AppResult<SessionInfo>;
    /// A new session in the tab of `tab`.
    fn replace(&self, tab: &str, request: &SessionRequest) -> AppResult<SessionInfo>;
    /// Ends a session's process; resolves once it is gone, or after `timeout_ms`.
    fn end(&self, tab: &str, timeout_ms: u64) -> BoxFuture<'static, ()>;
    /// The session is on the list.
    fn has_session(&self, id: &str) -> bool;
    /// Where Claude Code keeps the session's conversation, as its status line reported it.
    fn transcript_of(&self, id: &str) -> Option<String>;
}

fn refused(message: impl Into<String>) -> AppError {
    AppError::refused(message)
}

/// A text that JavaScript takes for true.
fn truthy(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// Where a step's new tab goes: next to the tab of the step before it, or where the run was started.
fn place_for<D: LauncherDeps + ?Sized>(deps: &D, run: &RunData, index: usize) -> Option<Place> {
    for step in run.steps[..index.min(run.steps.len())].iter().rev() {
        if let Some(tab) = truthy(step.session_id.as_deref()) {
            if deps.has_session(tab) {
                return Some(Place { group_id: None, after_id: Some(tab.to_owned()) });
            }
        }
    }
    deps.first_place(&run.id)
}

/// Opens the session of the step at `index` and sends it its message, the prompt having been
/// written to its file first. `cancelled` is asked right before a session is opened: true when
/// the run ended or moved on meanwhile (`The run was stopped.`).
pub async fn start_step<D: LauncherDeps + ?Sized>(
    deps: &D,
    run: &RunData,
    index: usize,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> AppResult<StartedStep> {
    let step = run.steps.get(index).ok_or_else(|| refused("The run has no such step."))?;
    let def = &step.def;
    let profile = deps.profile(&def.profile_id).ok_or_else(|| refused(format!("The account of step \"{}\" no longer exists.", def.name)))?;

    let dir = run_folder(&deps.runs_dir(), &run.id);
    let text = render_prompt(&def.prompt, &prompt_values(run, index));
    let prompt_file = write_step_prompt(&dir, index, &def.name, &text)?;

    let request = SessionRequest {
        profile_id: profile.id.clone(),
        cwd: run.cwd.clone(),
        kind: SessionKind::Claude,
        resume: None,
        name: Some(def.name.clone()),
        launch: Some(LaunchOptions {
            model: def.model.clone(),
            effort: def.effort.map(|e| e.as_str().to_owned()),
            permission_mode: def.permission_mode.map(|m| m.as_str().to_owned()),
            add_dirs: Some(vec![dir.to_string_lossy().into_owned()]),
            watch: Some(true),
        }),
        prompt: Some(launch_message(&run.name, &def.name, &prompt_file.to_string_lossy())),
    };
    let thread = run.threads.get(&step.thread).cloned().unwrap_or_default();
    let tab_of = || truthy(thread.session_id.as_deref()).filter(|tab| deps.has_session(tab)).map(str::to_owned);
    let open = |asked: &SessionRequest| -> AppResult<StartedStep> {
        if cancelled() {
            return Err(refused("The run was stopped."));
        }
        let info = match tab_of() {
            Some(tab) => deps.replace(&tab, asked)?,
            None => deps.create(asked, place_for(deps, run, index))?,
        };
        Ok(StartedStep { session_id: info.id, conversation_id: info.claude_session_id })
    };

    if step.thread == index {
        return open(&request);
    }

    let starter = run.steps.get(step.thread).map_or("", |s| s.def.name.as_str());
    let conversation = truthy(thread.conversation_id.as_deref())
        .map(str::to_owned)
        .ok_or_else(|| refused(format!("Step \"{starter}\" left no conversation to go on with.")))?;
    let holder = truthy(thread.profile_id.as_deref())
        .and_then(|id| deps.profile(id))
        .ok_or_else(|| refused(format!("The account that holds the conversation of \"{starter}\" no longer exists.")))?;
    let tab = tab_of();
    let reported = tab.as_deref().and_then(|t| deps.transcript_of(t));
    // the last lines are only on disk once the process is gone
    if let Some(tab) = &tab {
        deps.end(tab, END_BEFORE_RESUME_MS).await;
    }
    if holder.id != profile.id {
        let plan = plan_transfer(&deps.config_dir(&holder), &deps.config_dir(&profile), &conversation, reported.as_deref())
            .ok_or_else(|| refused(format!("The conversation of \"{starter}\" was not found on \"{}\".", holder.name)))?;
        if !can_replace(&plan).map_err(|e| AppError::io(e.to_string()))? {
            return Err(refused(format!(
                "\"{}\" already has the conversation of \"{starter}\" with a different continuation.",
                profile.name
            )));
        }
        copy_conversation(&plan).map_err(|e| AppError::io(e.to_string()))?;
    } else if find_transcript(&deps.config_dir(&profile), &conversation, reported.as_deref()).is_none() {
        return Err(refused(format!("The conversation of \"{starter}\" was not found.")));
    }
    open(&SessionRequest { resume: Some(SessionResume::Conversation(ResumeConversation { id: conversation })), ..request })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use mc_core::types::{Effort, Extra, HookSignal, PermissionMode, SessionStatus, StopError, Workflow, WorkflowStep};
    use mc_core::workflow::{new_run, reduce, RunEvent, RunSession, Thread};

    use super::*;

    const CONVERSATION: &str = "11111111-2222-3333-4444-555555555555";

    fn step(id: &str, name: &str, prompt: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            name: name.into(),
            profile_id: "main".into(),
            model: None,
            effort: None,
            permission_mode: None,
            continues: None,
            prompt: prompt.into(),
            gate: true,
            extra: Extra::new(),
        }
    }

    fn three() -> Workflow {
        let mut plan = step("plan", "Plan", "Plan: {{task}} in {{folder}}");
        plan.model = Some("fable".into());
        plan.permission_mode = Some(PermissionMode::Plan);
        let mut build = step("build", "Build it", "Build:\n{{previous.output}}");
        build.profile_id = "work".into();
        build.model = Some("opus".into());
        build.effort = Some(Effort::High);
        let mut review = step("review", "Review", "Review {{steps.Build it.output}} against {{steps.plan.output}}");
        review.continues = Some("plan".into());
        Workflow { id: "w1".into(), name: "Plan & \"ship\"".into(), steps: vec![plan, build, review], extra: Extra::new() }
    }

    fn play(mut run: RunData, events: &[RunEvent]) -> RunData {
        for event in events {
            run = reduce(&run, event, 0).run.into_owned();
        }
        run
    }

    fn hook(tab: &str, signal: HookSignal) -> RunEvent {
        RunEvent::Hook { session_id: tab.into(), conversation_id: None, signal }
    }

    fn answered(run: RunData, index: usize, tab: &str, text: &str, conversation: Option<&str>) -> RunData {
        play(
            run,
            &[
                RunEvent::Started { index, session_id: tab.into(), conversation_id: conversation.map(str::to_owned) },
                hook(tab, HookSignal::Prompt),
                hook(tab, HookSignal::Stop { text: text.into(), background: false }),
                RunEvent::Approve,
            ],
        )
    }

    fn tab(id: &str, profile_id: &str, replaces: Option<&str>) -> RunSession {
        RunSession { id: id.into(), profile_id: profile_id.into(), status: SessionStatus::Running, replaces: replaces.map(str::to_owned) }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Call {
        kind: &'static str,
        tab: Option<String>,
        request: Option<SessionRequest>,
        place: Option<Place>,
    }

    impl Call {
        fn line(&self) -> String {
            format!("{} {}", self.kind, self.tab.as_deref().unwrap_or("")).trim().to_owned()
        }
    }

    struct World {
        tmp: tempfile::TempDir,
        calls: Mutex<Vec<Call>>,
        tabs: Mutex<HashSet<String>>,
        n: AtomicUsize,
        no_profiles: AtomicBool,
    }

    impl World {
        fn new() -> Self {
            World {
                tmp: tempfile::tempdir().unwrap(),
                calls: Mutex::default(),
                tabs: Mutex::default(),
                n: AtomicUsize::new(0),
                no_profiles: AtomicBool::new(false),
            }
        }

        fn config_of(&self, id: &str) -> PathBuf {
            self.tmp.path().join("config").join(id)
        }

        fn runs(&self) -> PathBuf {
            self.tmp.path().join("runs")
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn lines(&self) -> Vec<String> {
            self.calls().iter().map(Call::line).collect()
        }

        fn info(&self, request: &SessionRequest) -> SessionInfo {
            let n = self.n.fetch_add(1, Ordering::SeqCst) + 1;
            let id = format!("tab{n}");
            self.tabs.lock().unwrap().insert(id.clone());
            let claude_session_id = match &request.resume {
                Some(SessionResume::Conversation(c)) => c.id.clone(),
                _ => format!("new-conversation-{n}"),
            };
            SessionInfo {
                id,
                profile_id: request.profile_id.clone(),
                kind: SessionKind::Claude,
                cwd: request.cwd.clone(),
                title: "t".into(),
                status: SessionStatus::Running,
                exit_code: None,
                started_at: 0,
                name: None,
                replaces: None,
                place: None,
                claude_session_id: Some(claude_session_id),
                model: None,
                context_pct: None,
                turn: None,
                moving: false,
                arrived: None,
            }
        }

        fn transcript(&self, config_dir: &Path, text: &str) -> PathBuf {
            let file = config_dir.join("projects").join("C--code-app").join(format!("{CONVERSATION}.jsonl"));
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, text).unwrap();
            file
        }
    }

    impl LauncherDeps for World {
        fn profile(&self, id: &str) -> Option<Profile> {
            if self.no_profiles.load(Ordering::SeqCst) || !["main", "work"].contains(&id) {
                return None;
            }
            let value = serde_json::json!({
                "id": id, "name": if id == "main" { "Main" } else { "Work" }, "slug": id, "color": "#fff",
                "isDefault": id == "main", "configDir": self.config_of(id).to_string_lossy(),
                "share": { "dirs": [], "files": [], "localSettingsKeys": [] }
            });
            Some(serde_json::from_value(value).unwrap())
        }
        fn config_dir(&self, profile: &Profile) -> PathBuf {
            PathBuf::from(profile.config_dir.clone().unwrap())
        }
        fn runs_dir(&self) -> PathBuf {
            self.runs()
        }
        fn first_place(&self, _run_id: &str) -> Option<Place> {
            Some(Place { group_id: Some("g2".into()), after_id: None })
        }
        fn create(&self, request: &SessionRequest, place: Option<Place>) -> AppResult<SessionInfo> {
            self.calls.lock().unwrap().push(Call { kind: "create", tab: None, request: Some(request.clone()), place });
            Ok(self.info(request))
        }
        fn replace(&self, tab: &str, request: &SessionRequest) -> AppResult<SessionInfo> {
            self.calls.lock().unwrap().push(Call { kind: "replace", tab: Some(tab.into()), request: Some(request.clone()), place: None });
            self.tabs.lock().unwrap().remove(tab);
            Ok(self.info(request))
        }
        fn end(&self, tab: &str, _timeout_ms: u64) -> BoxFuture<'static, ()> {
            self.calls.lock().unwrap().push(Call { kind: "end", tab: Some(tab.into()), request: None, place: None });
            Box::pin(async {})
        }
        fn has_session(&self, id: &str) -> bool {
            self.tabs.lock().unwrap().contains(id)
        }
        fn transcript_of(&self, _id: &str) -> Option<String> {
            None
        }
    }

    fn begun() -> RunData {
        play(new_run(&three(), "run1", "C:\\code\\app", "add a login", 0), &[RunEvent::Begin])
    }

    fn never() -> bool {
        false
    }

    async fn start(w: &World, run: &RunData, index: usize) -> AppResult<StartedStep> {
        start_step(w, run, index, &never).await
    }

    #[tokio::test]
    async fn opens_the_first_step_in_a_new_tab_with_its_prompt_in_a_file() {
        let w = World::new();
        let started = start(&w, &begun(), 0).await.unwrap();
        assert_eq!(started, StartedStep { session_id: "tab1".into(), conversation_id: Some("new-conversation-1".into()) });

        let dir = run_folder(&w.runs(), "run1");
        let file = dir.join("1-plan.prompt.md");
        assert_eq!(fs::read_to_string(&file).unwrap(), "Plan: add a login in C:\\code\\app");
        let calls = w.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].kind, "create");
        assert_eq!(calls[0].place, Some(Place { group_id: Some("g2".into()), after_id: None }));
        assert_eq!(
            calls[0].request,
            Some(SessionRequest {
                profile_id: "main".into(),
                cwd: "C:\\code\\app".into(),
                kind: SessionKind::Claude,
                resume: None,
                name: Some("Plan".into()),
                launch: Some(LaunchOptions {
                    model: Some("fable".into()),
                    effort: None,
                    permission_mode: Some("plan".into()),
                    add_dirs: Some(vec![dir.to_string_lossy().into_owned()]),
                    watch: Some(true),
                }),
                prompt: Some(launch_message("Plan & \"ship\"", "Plan", &file.to_string_lossy())),
            })
        );
        // the text of the prompt is nowhere in what goes on the command line
        assert!(!calls[0].request.as_ref().unwrap().prompt.as_ref().unwrap().contains("add a login"));
    }

    #[tokio::test]
    async fn opens_the_next_step_beside_the_one_before_it_with_that_steps_output_in_its_prompt() {
        let w = World::new();
        let first = start(&w, &begun(), 0).await.unwrap();
        let run = answered(begun(), 0, &first.session_id, "THE PLAN", Some(CONVERSATION));
        start(&w, &run, 1).await.unwrap();

        assert_eq!(fs::read_to_string(run_folder(&w.runs(), "run1").join("2-build-it.prompt.md")).unwrap(), "Build:\nTHE PLAN");
        let second = &w.calls()[1];
        assert_eq!(second.kind, "create");
        assert_eq!(second.place, Some(Place { group_id: None, after_id: Some("tab1".into()) }));
        let request = second.request.as_ref().unwrap();
        assert_eq!(request.profile_id, "work");
        let launch = request.launch.as_ref().unwrap();
        assert_eq!((launch.model.as_deref(), launch.effort.as_deref(), launch.permission_mode.as_deref()), (Some("opus"), Some("high"), None));
        assert_eq!(request.resume, None);
    }

    async fn at_review(w: &World) -> RunData {
        let first = start(w, &begun(), 0).await.unwrap();
        let run = answered(begun(), 0, &first.session_id, "THE PLAN", Some(CONVERSATION));
        let second = start(w, &run, 1).await.unwrap();
        let run = answered(run, 1, &second.session_id, "BUILT", None);
        w.calls.lock().unwrap().clear();
        run
    }

    fn resumes(request: &SessionRequest) -> Option<&str> {
        match &request.resume {
            Some(SessionResume::Conversation(c)) => Some(c.id.as_str()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn goes_on_with_an_earlier_conversation_in_that_conversations_tab() {
        let w = World::new();
        w.transcript(&w.config_of("main"), "{\"type\":\"user\"}\n");
        let run = at_review(&w).await;
        let started = start(&w, &run, 2).await.unwrap();

        assert_eq!(w.lines(), ["end tab1", "replace tab1"]);
        let request = w.calls()[1].request.clone().unwrap();
        assert_eq!((request.profile_id.as_str(), request.name.as_deref(), resumes(&request)), ("main", Some("Review"), Some(CONVERSATION)));
        assert_eq!(started, StartedStep { session_id: "tab3".into(), conversation_id: Some(CONVERSATION.into()) });
        assert_eq!(fs::read_to_string(run_folder(&w.runs(), "run1").join("3-review.prompt.md")).unwrap(), "Review BUILT against THE PLAN");
    }

    #[tokio::test]
    async fn opens_a_new_tab_for_it_when_the_old_one_was_closed() {
        let w = World::new();
        w.transcript(&w.config_of("main"), "{\"type\":\"user\"}\n");
        let run = at_review(&w).await;
        w.tabs.lock().unwrap().remove("tab1");
        let run = play(run, &[RunEvent::Sessions { list: vec![tab("tab2", "work", None)] }]);
        start(&w, &run, 2).await.unwrap();
        let calls = w.calls();
        assert_eq!(w.lines(), ["create"]);
        assert_eq!(calls[0].place, Some(Place { group_id: None, after_id: Some("tab2".into()) }));
        assert_eq!(resumes(calls[0].request.as_ref().unwrap()), Some(CONVERSATION));
    }

    #[tokio::test]
    async fn copies_the_conversation_first_when_it_lives_on_another_account() {
        let w = World::new();
        // the plan was moved to the other account while the run was under way
        w.transcript(&w.config_of("work"), "{\"type\":\"user\",\"moved\":true}\n");
        let run = at_review(&w).await;
        let run = play(run, &[RunEvent::Sessions { list: vec![tab("tab9", "work", Some("tab1")), tab("tab2", "work", None)] }]);
        {
            let mut tabs = w.tabs.lock().unwrap();
            tabs.remove("tab1");
            tabs.insert("tab9".into());
        }
        start(&w, &run, 2).await.unwrap();

        assert_eq!(w.lines(), ["end tab9", "replace tab9"]);
        let request = w.calls()[1].request.clone().unwrap();
        assert_eq!((request.profile_id.as_str(), resumes(&request)), ("main", Some(CONVERSATION)));
        let copied = w.config_of("main").join("projects").join("C--code-app").join(format!("{CONVERSATION}.jsonl"));
        assert_eq!(fs::read_to_string(copied).unwrap(), "{\"type\":\"user\",\"moved\":true}\n");
    }

    #[tokio::test]
    async fn says_what_is_missing_instead_of_opening_a_session_that_cannot_work() {
        let w = World::new();
        // no conversation file anywhere
        let run = at_review(&w).await;
        let missing = start(&w, &run, 2).await.unwrap_err();
        assert_eq!(missing.to_string(), "The conversation of \"Plan\" was not found.");
        assert!(w.calls().iter().all(|c| c.kind == "end"));

        w.no_profiles.store(true, Ordering::SeqCst);
        let gone = start(&w, &begun(), 0).await.unwrap_err();
        assert_eq!(gone.to_string(), "The account of step \"Plan\" no longer exists.");
        w.no_profiles.store(false, Ordering::SeqCst);

        let mut unknown = play(run, &[RunEvent::Sessions { list: vec![] }]);
        unknown.threads.insert(0, Thread { session_id: None, conversation_id: None, profile_id: Some("main".into()) });
        let left = start(&w, &unknown, 2).await.unwrap_err();
        assert!(left.to_string().contains("left no conversation to go on with"), "{left}");
    }

    #[tokio::test]
    async fn opens_nothing_once_the_run_was_stopped() {
        let w = World::new();
        let stopped = start_step(&w, &begun(), 0, &|| true).await.unwrap_err();
        assert_eq!(stopped.to_string(), "The run was stopped.");
        assert!(w.calls().is_empty());
    }

    #[tokio::test]
    async fn takes_over_the_same_tab_when_a_step_is_tried_again() {
        let w = World::new();
        let first = start(&w, &begun(), 0).await.unwrap();
        let run = play(
            begun(),
            &[
                RunEvent::Started { index: 0, session_id: first.session_id.clone(), conversation_id: Some("c1".into()) },
                hook(&first.session_id, HookSignal::Failure { reason: "x".into(), error: StopError::Unknown, text: String::new() }),
                RunEvent::Retry,
            ],
        );
        start(&w, &run, 0).await.unwrap();
        assert_eq!(w.lines(), ["create", "replace tab1"]);
        // a conversation of its own again, not the one that failed
        assert_eq!(w.calls()[1].request.as_ref().unwrap().resume, None);
    }
}
