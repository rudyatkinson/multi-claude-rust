use std::sync::mpsc;

use mc_core::types::{ResumeConversation, ResumeMode, SessionResume, TurnKind};

use super::*;

#[derive(Default)]
struct Recorder {
    data: Mutex<Vec<PtyData>>,
    lists: Mutex<Vec<SessionsChanged>>,
}

impl Outlet for Recorder {
    fn data(&self, payload: PtyData) {
        lock(&self.data).push(payload);
    }
    fn sessions(&self, payload: SessionsChanged) {
        lock(&self.lists).push(payload);
    }
}

impl Recorder {
    fn lists(&self) -> Vec<SessionsChanged> {
        lock(&self.lists).clone()
    }
    fn ids(&self) -> Vec<Vec<String>> {
        self.lists().iter().map(|l| l.sessions.iter().map(|s| s.id.clone()).collect()).collect()
    }
}

fn manager() -> (Arc<Recorder>, SessionManager) {
    let recorder = Arc::new(Recorder::default());
    let manager = SessionManager::with_outlet(recorder.clone(), std::env::temp_dir());
    (recorder, manager)
}

/// The cleanup calls, as (profile, session).
fn record_cleanups(manager: &SessionManager) -> Arc<Mutex<Vec<(String, String)>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    manager.set_cleanup_handler(Box::new(move |profile, session| lock(&seen).push((profile.to_owned(), session.to_owned()))));
    calls
}

fn request(cwd: &str, kind: SessionKind) -> SessionRequest {
    SessionRequest { profile_id: "work".into(), cwd: cwd.into(), kind, resume: None, name: None, launch: None, prompt: None }
}

fn info(id: &str, kind: SessionKind) -> SessionInfo {
    SessionInfo {
        id: id.into(),
        profile_id: "work".into(),
        kind,
        cwd: "/proj".into(),
        title: "proj".into(),
        status: SessionStatus::Running,
        exit_code: None,
        started_at: 1,
        name: None,
        replaces: None,
        place: None,
        claude_session_id: None,
        model: None,
        context_pct: None,
        turn: None,
        moving: false,
        arrived: None,
        keep_awake: false,
        slept_at: None,
    }
}

/// A session in the list without a process, as one whose process has not been started.
fn fake(manager: &SessionManager, id: &str, kind: SessionKind, intro: Option<(Option<Place>, Option<String>)>) {
    let (_tx, exited) = watch::channel(false);
    let live = Live {
        info: info(id, kind),
        request: request("/proj", kind),
        transcript_path: None,
        pty: None,
        ending: false,
        stream: Stream::new(id.into(), manager.inner.outlet.clone()),
        exited,
        intro,
        last_input_at: 1,
        size: None,
        to_sleep: false,
    };
    let mut t = manager.table();
    t.order.push(id.into());
    t.live.insert(id.into(), live);
}

fn temp_folder() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("mc-sessions-").tempdir().unwrap()
}

fn uuid_like(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok()
}

// ---------- plans ----------

#[test]
fn a_claude_tab_is_planned_as_spawn_built_it() {
    let (_rec, manager) = manager();
    let dir = temp_folder();
    let cwd = dir.path().to_string_lossy().into_owned();
    let mut req = request(&cwd, SessionKind::Claude);
    req.name = Some("Build".into());
    req.prompt = Some("hello".into());
    let mut asked = None;
    let plan = manager
        .plan(&req, "/bin/claude", Some("ignored@for.claude"), BTreeMap::from([("A".to_owned(), "1".to_owned())]), |id, hooks| {
            asked = Some((id.to_owned(), hooks));
            Ok(Some(PathBuf::from("S.json")))
        })
        .unwrap();
    let (id, hooks) = asked.unwrap();
    assert_eq!(id, plan.id);
    assert!(uuid_like(&plan.id));
    assert_eq!(hooks, HookNeed::Wanted);
    assert_eq!(plan.program, "/bin/claude");
    let conversation = plan.claude_session_id.clone().unwrap();
    assert!(uuid_like(&conversation));
    assert_eq!(plan.args, vec!["--settings", "S.json", "--session-id", &conversation, "--", "hello"]);
    assert_eq!(plan.title, dir.path().file_name().unwrap().to_string_lossy());
    assert_eq!(plan.name.as_deref(), Some("Build"));
    assert_eq!(plan.env["A"], "1");
    // the first message is not kept for a restart
    assert_eq!(plan.request.prompt, None);
    assert_eq!(plan.request.name.as_deref(), Some("Build"));
    assert_eq!(plan.place, None);
    // the token stays out of debug output
    assert!(format!("{plan:?}").contains("1 variables"));
}

#[test]
fn a_workflow_step_must_report_its_turns() {
    let (_rec, manager) = manager();
    let dir = temp_folder();
    let mut req = request(&dir.path().to_string_lossy(), SessionKind::Claude);
    req.launch = Some(LaunchOptions { watch: Some(true), ..LaunchOptions::default() });
    req.resume = Some(SessionResume::Conversation(ResumeConversation { id: "abc".into() }));
    let mut need = None;
    let plan = manager
        .plan(&req, "/bin/claude", None, BTreeMap::new(), |_, hooks| {
            need = Some(hooks);
            Ok(None)
        })
        .unwrap();
    assert_eq!(need, Some(HookNeed::Required));
    assert_eq!(plan.args, vec!["--resume", "abc"]);
    assert_eq!(plan.claude_session_id.as_deref(), Some("abc"));
}

#[test]
fn a_login_tab_runs_auth_login() {
    let (_rec, manager) = manager();
    let dir = temp_folder();
    let mut req = request(&dir.path().to_string_lossy(), SessionKind::Login);
    req.name = Some("not for a login".into());
    let plan = manager.plan(&req, "/bin/claude", Some("a@b.co"), BTreeMap::new(), |_, _| panic!("a login has no settings")).unwrap();
    assert_eq!(plan.args, vec!["auth", "login", "--claudeai", "--email", "a@b.co"]);
    assert_eq!(plan.title, "Login");
    assert_eq!(plan.name, None);
    assert_eq!(plan.claude_session_id, None);
    let plain = manager.plan(&req, "/bin/claude", Some(""), BTreeMap::new(), |_, _| Ok(None)).unwrap();
    assert_eq!(plain.args, vec!["auth", "login", "--claudeai"]);
}

#[test]
fn a_missing_folder_is_refused_before_anything_is_prepared() {
    let (_rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let req = request("/surely/not/here/mc", SessionKind::Claude);
    let err = manager.plan(&req, "/bin/claude", None, BTreeMap::new(), |_, _| panic!("not prepared")).unwrap_err();
    assert_eq!(err.to_string(), "Folder not found: /surely/not/here/mc");
    assert!(lock(&cleanups).is_empty());
}

#[test]
fn failed_settings_or_arguments_are_cleaned_up() {
    let (_rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let req = request(&dir.path().to_string_lossy(), SessionKind::Claude);
    let err = manager
        .plan(&req, "/bin/claude", None, BTreeMap::new(), |_, _| {
            Err(AppError::refused("The app cannot receive the signals of this session right now. Restart Multi Claude and try again."))
        })
        .unwrap_err();
    assert!(err.to_string().starts_with("The app cannot receive"));
    assert_eq!(lock(&cleanups).len(), 1);
    assert_eq!(lock(&cleanups)[0].0, "work");

    let mut bad = req.clone();
    bad.launch = Some(LaunchOptions { model: Some("--print".into()), ..LaunchOptions::default() });
    let err = manager.plan(&bad, "/bin/claude", None, BTreeMap::new(), |_, _| Ok(None)).unwrap_err();
    assert_eq!(err.to_string(), "Invalid model: --print");
    assert_eq!(lock(&cleanups).len(), 2);
}

#[cfg(windows)]
#[test]
fn a_first_message_needs_the_native_executable() {
    let (_rec, manager) = manager();
    let dir = temp_folder();
    let mut req = request(&dir.path().to_string_lossy(), SessionKind::Claude);
    req.prompt = Some("go".into());
    let err = manager.plan(&req, "C:\\npm\\claude.cmd", None, BTreeMap::new(), |_, _| panic!("not prepared")).unwrap_err();
    assert!(err.to_string().starts_with("A session that starts with a message needs the native claude executable"));
    // without a message the shim goes through cmd.exe
    req.prompt = None;
    req.resume = Some(SessionResume::Mode(ResumeMode::Last));
    let plan = manager.plan(&req, "C:\\npm\\claude.cmd", None, BTreeMap::new(), |_, _| Ok(None)).unwrap();
    assert!(plan.program.to_ascii_lowercase().ends_with("cmd.exe"));
    assert_eq!(plan.args, vec!["/d", "/c", "C:\\npm\\claude.cmd", "-c"]);
}

#[test]
fn titles_are_the_folder_name() {
    assert_eq!(title_of("/home/u/proj"), "proj");
    assert_eq!(title_of("/home/u/proj/"), "proj");
    assert_eq!(title_of("/"), "/");
    if cfg!(windows) {
        assert_eq!(title_of("C:\\Users\\u\\proj"), "proj");
        assert_eq!(title_of("C:\\Users\\u\\proj\\"), "proj");
        assert_eq!(title_of("C:\\"), "C:\\");
        assert_eq!(title_of("C:/work/x"), "x");
    }
}

// ---------- the list ----------

#[test]
fn place_and_replaces_travel_with_one_list_only() {
    let (rec, manager) = manager();
    let place = Place { group_id: Some("g1".into()), after_id: None };
    fake(&manager, "a", SessionKind::Claude, Some((Some(place.clone()), Some("old".into()))));
    manager.announce();
    manager.announce();
    let lists = rec.lists();
    assert_eq!(lists[0].sessions[0].place, Some(place));
    assert_eq!(lists[0].sessions[0].replaces.as_deref(), Some("old"));
    assert_eq!(lists[1].sessions[0].place, None);
    assert_eq!(lists[1].sessions[0].replaces, None);
    // list() never carries them
    assert_eq!(manager.list()[0].place, None);
}

#[test]
fn closing_announces_once_and_cleans_up_each() {
    let (rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    for id in ["a", "b", "c"] {
        fake(&manager, id, SessionKind::Claude, None);
    }
    manager.close_many(&["a", "c", "nope"]);
    assert_eq!(rec.ids(), vec![vec!["b".to_owned()]]);
    assert_eq!(lock(&cleanups).iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
    // nothing to close: nothing to announce
    manager.close("nope");
    assert_eq!(rec.lists().len(), 1);
    manager.close("b");
    assert_eq!(rec.ids().last().unwrap(), &Vec::<String>::new());
}

#[test]
fn closing_an_account_closes_its_tabs() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    fake(&manager, "b", SessionKind::Claude, None);
    manager.table().live.get_mut("b").unwrap().info.profile_id = "home".into();
    manager.close_for_profile("work");
    assert_eq!(rec.ids(), vec![vec!["b".to_owned()]]);
    manager.kill_all();
    assert!(manager.list().is_empty());
    // the app quits: no announcement
    assert_eq!(rec.lists().len(), 1);
}

#[test]
fn renaming_names_claude_tabs_only_and_only_on_change() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    fake(&manager, "l", SessionKind::Login, None);
    manager.rename("a", Some("Build".into()));
    manager.rename("a", Some("Build".into()));
    manager.rename("l", Some("Login tab".into()));
    assert_eq!(rec.lists().len(), 1);
    assert_eq!(manager.get("a").unwrap().name.as_deref(), Some("Build"));
    assert_eq!(manager.request_of("a").unwrap().name.as_deref(), Some("Build"));
    assert_eq!(manager.get("l").unwrap().name, None);
    manager.rename("a", None);
    assert_eq!(manager.get("a").unwrap().name, None);
    assert_eq!(rec.lists().len(), 2);
}

#[test]
fn turns_moves_and_arrivals_are_announced_when_they_change() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    let working = Turn { at: 5, kind: TurnKind::Working };
    manager.set_turn("a", Some(working.clone()));
    manager.set_turn("a", Some(working.clone()));
    assert_eq!(rec.lists().len(), 1);
    assert_eq!(manager.get("a").unwrap().turn, Some(working.clone()));
    manager.set_moving("a", true);
    manager.set_moving("a", true);
    assert_eq!(rec.lists().len(), 2);
    assert!(manager.get("a").unwrap().moving);
    manager.set_moving("a", false);
    assert_eq!(rec.lists().len(), 3);
    manager.set_arrived("a", Arrival { from: "home".into(), at: 9, limit: "5-hour limit".into(), picked_up: true, auto: false });
    assert_eq!(rec.lists().len(), 4);
    assert_eq!(manager.get("a").unwrap().arrived.unwrap().from, "home");
    // an ended session takes no turn
    manager.table().live.get_mut("a").unwrap().info.status = SessionStatus::Exited;
    manager.set_turn("a", None);
    assert_eq!(rec.lists().len(), 4);
    manager.set_turn("nope", Some(working));
    assert_eq!(rec.lists().len(), 4);
}

#[test]
fn only_one_caller_gets_to_mark_a_session_as_moving() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    assert!(manager.try_set_moving("a"));
    assert!(!manager.try_set_moving("a"));
    assert!(manager.get("a").unwrap().moving);
    // the caller announces it when the window is to show it
    assert!(rec.lists().is_empty());
    manager.set_moving("a", false);
    assert!(manager.try_set_moving("a"));
    assert!(!manager.try_set_moving("nope"));
}

#[test]
fn a_replacement_for_a_tab_that_is_gone_is_cleaned_up() {
    let (_rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let plan = manager.plan(&request(&dir.path().to_string_lossy(), SessionKind::Login), "/bin/claude", None, BTreeMap::new(), |_, _| Ok(None)).unwrap();
    let id = plan.id.clone();
    assert_eq!(manager.replace("gone", plan).unwrap_err().to_string(), "Session not found");
    assert_eq!(*lock(&cleanups), vec![("work".to_owned(), id)]);
}

#[test]
fn the_status_line_fills_in_what_it_knows() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    let meta = SessionMeta { claude_session_id: Some("c1".into()), transcript_path: Some("/t/c1.jsonl".into()), model: Some("Opus".into()), context_pct: Some(12) };
    manager.set_meta("a", &meta);
    manager.set_meta("a", &meta);
    assert_eq!(rec.lists().len(), 1);
    manager.set_meta("a", &SessionMeta { context_pct: Some(13), ..SessionMeta::default() });
    let info = manager.get("a").unwrap();
    assert_eq!((info.claude_session_id.as_deref(), info.model.as_deref(), info.context_pct), (Some("c1"), Some("Opus"), Some(13)));
    assert_eq!(manager.transcript_of("a").as_deref(), Some("/t/c1.jsonl"));
    assert_eq!(rec.lists().len(), 2);
    // a transcript path alone changes nothing the window sees
    manager.set_meta("a", &SessionMeta { transcript_path: Some("/t/c2.jsonl".into()), ..SessionMeta::default() });
    assert_eq!(rec.lists().len(), 2);
    assert_eq!(manager.transcript_of("a").as_deref(), Some("/t/c2.jsonl"));
}

#[test]
fn saved_tabs_are_the_running_and_the_sleeping_claude_tabs() {
    let (_rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    fake(&manager, "l", SessionKind::Login, None);
    fake(&manager, "x", SessionKind::Claude, None);
    fake(&manager, "z", SessionKind::Claude, None);
    manager.table().live.get_mut("x").unwrap().info.status = SessionStatus::Exited;
    manager.table().live.get_mut("z").unwrap().info.status = SessionStatus::Sleeping;
    manager.rename("a", Some("Build".into()));
    let tabs = manager.saved_tabs();
    assert_eq!(tabs.iter().map(|(id, tab)| (id.as_str(), tab.asleep)).collect::<Vec<_>>(), [("a", false), ("z", true)]);
    assert_eq!(
        tabs[0].1,
        SavedTab { profile_id: "work".into(), cwd: "/proj".into(), claude_session_id: None, transcript_path: None, name: Some("Build".into()), asleep: false }
    );
    assert!(manager.has_running());
    // a sleeping tab is not a running one
    for id in ["a", "l"] {
        manager.table().live.get_mut(id).unwrap().info.status = SessionStatus::Exited;
    }
    assert!(!manager.has_running());
}

#[test]
fn the_changed_handler_sees_every_list_and_may_call_back() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    let handle = manager.clone();
    let seen = Arc::new(Mutex::new(Vec::<usize>::new()));
    let counted = seen.clone();
    manager.set_changed_handler(Box::new(move |list| {
        lock(&counted).push(list.len());
        if lock(&counted).len() == 1 {
            // a change made while the list is handled is announced right after it
            handle.rename("a", Some("named".into()));
            return Some(Layout::default());
        }
        None
    }));
    manager.announce();
    assert_eq!(*lock(&seen), vec![1, 1]);
    let lists = rec.lists();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].layout, Some(Layout::default()));
    assert_eq!(lists[0].sessions[0].name, None);
    assert_eq!(lists[1].layout, None);
    assert_eq!(lists[1].sessions[0].name.as_deref(), Some("named"));
}

#[test]
fn a_layout_given_to_the_announcement_is_sent_with_it() {
    let (rec, manager) = manager();
    manager.set_changed_handler(Box::new(|_| None));
    manager.emit_changed(Some(Layout::default()));
    manager.emit_changed(None);
    let lists = rec.lists();
    assert_eq!(lists[0].layout, Some(Layout::default()));
    assert_eq!(lists[1].layout, None);
}

#[test]
fn sessions_without_a_process_take_no_input_and_end_at_once() {
    let (_rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    assert!(!manager.write("a", "x"));
    assert!(!manager.write("nope", "x"));
    manager.resize("a", 80, 24);
    manager.kill("a");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let started = std::time::Instant::now();
    rt.block_on(manager.end("a", 5_000));
    rt.block_on(manager.end("nope", 5_000));
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// What happened to a session's pty, in order: `close` (its console) and `kill` (its process).
type PtyLog = Arc<Mutex<Vec<&'static str>>>;

#[derive(Debug)]
struct FakeKiller(PtyLog);

impl ChildKiller for FakeKiller {
    fn kill(&mut self) -> std::io::Result<()> {
        lock(&self.0).push("kill");
        Ok(())
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(FakeKiller(self.0.clone()))
    }
}

struct FakeConsole(PtyLog);

impl Console for FakeConsole {
    fn resize(&self, _size: PtySize) -> Result<(), String> {
        Ok(())
    }
}

impl Drop for FakeConsole {
    fn drop(&mut self) {
        lock(&self.0).push("close");
    }
}

/// A running session with a pty that only records what is done to it, and the switch that says
/// its process is gone.
fn fake_running(manager: &SessionManager, id: &str) -> (PtyLog, watch::Sender<bool>) {
    let (log, exited, _input) = fake_running_with_input(manager, id);
    (log, exited)
}

/// [`fake_running`], with what is written to its terminal.
fn fake_running_with_input(manager: &SessionManager, id: &str) -> (PtyLog, watch::Sender<bool>, mpsc::Receiver<Vec<u8>>) {
    fake(manager, id, SessionKind::Claude, None);
    let log = PtyLog::default();
    let (input, queue) = mpsc::channel();
    let (exited_tx, exited) = watch::channel(false);
    let mut t = manager.table();
    let live = t.live.get_mut(id).unwrap();
    live.pty = Some(Pty { master: Some(Box::new(FakeConsole(log.clone()))), input, killer: Box::new(FakeKiller(log.clone())) });
    live.exited = exited;
    (log, exited_tx, queue)
}

/// What the waiter thread does once the fake process of `id` is gone.
fn fake_exit(manager: &SessionManager, id: &str, exited: &watch::Sender<bool>) {
    let stream = manager.table().live[id].stream.clone();
    stream.end();
    manager.finish_exit(id, "work", &stream, Some(0));
    exited.send_replace(true);
}

/// The pty's log once it has `len` entries (the console is closed on a thread of its own).
fn log_of(log: &PtyLog, len: usize) -> Vec<&'static str> {
    let started = std::time::Instant::now();
    while lock(log).len() < len && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(2));
    }
    lock(log).clone()
}

async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[cfg(windows)]
#[tokio::test(start_paused = true)]
async fn ending_closes_the_console_and_kills_only_a_process_that_stays() {
    let (_rec, manager) = manager();
    // a process that ends once its console is closed is never killed
    let (log, exited) = fake_running(&manager, "a");
    let ending = tokio::spawn(manager.end("a", 5_000));
    // its timers start when it first runs
    settle().await;
    assert_eq!(log_of(&log, 1), ["close"]);
    tokio::time::advance(Duration::from_millis(1_000)).await;
    settle().await;
    assert_eq!(*lock(&log), ["close"]);
    exited.send_replace(true);
    ending.await.unwrap();
    assert_eq!(*lock(&log), ["close"]);

    // one that stays is killed after the grace
    let (log, exited) = fake_running(&manager, "b");
    let ending = tokio::spawn(manager.end("b", 5_000));
    settle().await;
    assert_eq!(log_of(&log, 1), ["close"]);
    tokio::time::advance(END_GRACE - Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(*lock(&log), ["close"]);
    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(*lock(&log), ["close", "kill"]);
    exited.send_replace(true);
    ending.await.unwrap();

    // a timeout shorter than the grace kills at the timeout, and the end is not waited for after it
    let (log, _exited) = fake_running(&manager, "c");
    let ending = tokio::spawn(manager.end("c", 500));
    settle().await;
    assert_eq!(log_of(&log, 1), ["close"]);
    tokio::time::advance(Duration::from_millis(500)).await;
    settle().await;
    assert_eq!(*lock(&log), ["close", "kill"]);
    assert!(ending.is_finished());
}

#[test]
fn closing_a_tab_kills_its_process_at_once() {
    let (_rec, manager) = manager();
    let (log, _exited) = fake_running(&manager, "a");
    manager.close("a");
    let expected: &[&str] = if cfg!(windows) { &["kill", "close"] } else { &["kill"] };
    assert_eq!(log_of(&log, expected.len()), expected);
    let (log, _exited) = fake_running(&manager, "b");
    manager.kill_all();
    assert_eq!(log_of(&log, expected.len()), expected);
}

#[test]
fn attaching_gives_the_replay() {
    let (rec, manager) = manager();
    fake(&manager, "a", SessionKind::Claude, None);
    let stream = manager.table().live["a"].stream.clone();
    stream.push(b"hello");
    let attached = manager.attach("a").unwrap();
    assert_eq!((attached.replay.as_str(), attached.seq), ("hello", 5));
    assert_eq!(lock(&rec.data).len(), 1);
    assert!(manager.attach("nope").is_none());
}

// ---------- sleeping ----------

#[tokio::test]
async fn a_session_falls_asleep_under_its_own_id_and_keeps_its_tab() {
    let (rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let ended = exits(&manager);
    let (log, exited, _input) = fake_running_with_input(&manager, "a");
    {
        let mut t = manager.table();
        let live = t.live.get_mut("a").unwrap();
        live.info.name = Some("Build".into());
        live.info.claude_session_id = Some("c1".into());
        live.info.turn = Some(Turn { at: 5, kind: TurnKind::Done { background: false } });
        live.transcript_path = Some("/t/c1.jsonl".into());
    }
    let stream = manager.table().live["a"].stream.clone();
    stream.push(b"what it said");
    stream.flush();

    let ending = manager.sleep("a", 3_000).unwrap();
    // the mark keeps a move, a restart or a second sleep out, and the window does not see it
    assert!(manager.get("a").unwrap().moving);
    assert!(!manager.try_set_moving("a"));
    assert!(manager.sleep("a", 3_000).is_err());
    assert!(rec.lists().is_empty());
    let asked: &[&str] = if cfg!(windows) { &["close"] } else { &["kill"] };
    assert_eq!(log_of(&log, 1), asked);

    // the process goes
    fake_exit(&manager, "a", &exited);
    ending.await;
    let info = manager.get("a").unwrap();
    assert_eq!(info.id, "a");
    assert_eq!(info.status, SessionStatus::Sleeping);
    assert!(info.slept_at.is_some());
    assert!(!info.moving);
    assert_eq!((info.turn, info.exit_code), (None, None));
    assert_eq!((info.name.as_deref(), info.claude_session_id.as_deref()), (Some("Build"), Some("c1")));
    assert_eq!(manager.transcript_of("a").as_deref(), Some("/t/c1.jsonl"));
    // announced once, as asleep; its files went as for any end, but nobody hears of an exit
    let lists = rec.lists();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].sessions[0].status, SessionStatus::Sleeping);
    assert_eq!(*lock(&cleanups), [("work".to_owned(), "a".to_owned())]);
    assert!(ended.try_recv().is_err());

    // it keeps what it showed, takes no input, and keeps the size for its next process
    let attached = manager.attach("a").unwrap();
    assert_eq!((attached.replay.as_str(), attached.seq), ("what it said", 12));
    assert!(!manager.write("a", "x"));
    manager.resize("a", 90, 33);
    assert_eq!(manager.table().live["a"].size, Some((90, 33)));
    assert!(!manager.has_running());
    let saved = manager.saved_tabs();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].1.asleep);
    assert_eq!(saved[0].1.claude_session_id.as_deref(), Some("c1"));

    // a sleeping tab does not sleep again; closing it has nothing left to clean up
    assert!(manager.sleep("a", 3_000).is_err());
    manager.close("a");
    assert!(manager.get("a").is_none());
    assert_eq!(lock(&cleanups).len(), 1);
}

#[test]
fn only_a_running_claude_tab_can_sleep() {
    let (_rec, manager) = manager();
    // no process
    fake(&manager, "idle", SessionKind::Claude, None);
    assert!(manager.sleep("idle", 1).is_err());
    let (_log, _exited) = fake_running(&manager, "login");
    manager.table().live.get_mut("login").unwrap().info.kind = SessionKind::Login;
    assert!(manager.sleep("login", 1).is_err());
    let (_log, _exited) = fake_running(&manager, "moving");
    assert!(manager.try_set_moving("moving"));
    assert!(manager.sleep("moving", 1).is_err());
    assert!(manager.sleep("nope", 1).is_err());
    // none of them was touched
    for id in ["idle", "login", "moving"] {
        assert!(!manager.table().live[id].to_sleep, "{id}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_process_that_outlasts_the_wait_falls_asleep_once_it_ends() {
    let (_rec, manager) = manager();
    let (_log, exited) = fake_running(&manager, "a");
    let ending = tokio::spawn(manager.sleep("a", 500).unwrap());
    settle().await;
    tokio::time::advance(Duration::from_millis(500)).await;
    settle().await;
    ending.await.unwrap();
    // still there: the mark comes off, so that the tab can be closed or restarted
    let info = manager.get("a").unwrap();
    assert_eq!(info.status, SessionStatus::Running);
    assert!(!info.moving);
    fake_exit(&manager, "a", &exited);
    assert_eq!(manager.get("a").unwrap().status, SessionStatus::Sleeping);
}

#[test]
fn an_ended_process_that_was_not_put_to_sleep_ends_its_tab() {
    let (_rec, manager) = manager();
    let ended = exits(&manager);
    let (_log, exited) = fake_running(&manager, "a");
    fake_exit(&manager, "a", &exited);
    assert_eq!(manager.get("a").unwrap().status, SessionStatus::Exited);
    assert_eq!(manager.get("a").unwrap().slept_at, None);
    assert_eq!(ended.try_recv().unwrap().id, "a");
}

#[test]
fn a_tab_of_the_last_run_comes_back_asleep_without_a_process() {
    let (rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let mut req = request("/proj/app", SessionKind::Claude);
    req.name = Some("Build".into());
    req.prompt = Some("never said".into());
    let info = manager.create_sleeping(SleepingTab {
        id: "s".into(),
        profile_id: "work".into(),
        cwd: "/proj/app".into(),
        name: Some("Build".into()),
        claude_session_id: Some("c1".into()),
        transcript_path: Some("/t/c1.jsonl".into()),
        request: req,
    });
    assert_eq!((info.id.as_str(), info.status, info.title.as_str()), ("s", SessionStatus::Sleeping, "app"));
    assert!(info.slept_at.is_some());
    // announced with the other restored tabs
    assert!(rec.lists().is_empty());
    let attached = manager.attach("s").unwrap();
    assert_eq!((attached.replay.as_str(), attached.seq), ("", 0));
    assert_eq!(manager.request_of("s").unwrap().prompt, None);
    assert_eq!(manager.transcript_of("s").as_deref(), Some("/t/c1.jsonl"));
    assert_eq!(
        manager.saved_tabs()[0].1,
        SavedTab {
            profile_id: "work".into(),
            cwd: "/proj/app".into(),
            claude_session_id: Some("c1".into()),
            transcript_path: Some("/t/c1.jsonl".into()),
            name: Some("Build".into()),
            asleep: true,
        }
    );
    assert!(!manager.write("s", "x"));
    assert!(!manager.has_running());
    manager.close("s");
    assert!(lock(&cleanups).is_empty());
}

#[test]
fn waking_a_tab_that_is_not_asleep_is_refused_and_cleaned_up() {
    let (_rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let (_log, _exited) = fake_running(&manager, "a");
    let mut plan = shell_plan("/proj", "echo");
    plan.id = "a".into();
    assert_eq!(manager.wake(plan.clone()).unwrap_err().to_string(), "This tab is not asleep.");
    plan.id = "gone".into();
    assert_eq!(manager.wake(plan).unwrap_err().to_string(), "Session not found");
    // the files made for the wake go again (`wake_session` plans one only for a tab that sleeps)
    assert_eq!(*lock(&cleanups), [("work".to_owned(), "a".to_owned()), ("work".to_owned(), "gone".to_owned())]);
    assert_eq!(manager.get("a").unwrap().status, SessionStatus::Running);
}

#[test]
fn typing_counts_as_activity_and_keep_awake_is_announced_when_it_changes() {
    let (rec, manager) = manager();
    let (_log, _exited, queue) = fake_running_with_input(&manager, "a");
    fake(&manager, "l", SessionKind::Login, None);
    let started = manager.activity()[0].last_input_at;
    // the terminal's report of its focus is not typing
    assert!(manager.write("a", "\x1b[I"));
    assert_eq!(manager.activity()[0].last_input_at, started);
    std::thread::sleep(Duration::from_millis(5));
    assert!(manager.write("a", "hi"));
    assert!(manager.activity()[0].last_input_at > started);
    assert_eq!(queue.try_iter().collect::<Vec<_>>(), [b"\x1b[I".to_vec(), b"hi".to_vec()]);
    // the stream knows when output came
    let stream = manager.table().live["a"].stream.clone();
    stream.push(b"out");
    assert_eq!(manager.activity()[0].last_output_at, stream.last_output_at());
    assert_eq!(manager.activity().iter().map(|a| a.info.id.as_str()).collect::<Vec<_>>(), ["a", "l"]);

    manager.set_keep_awake("a", true).unwrap();
    manager.set_keep_awake("a", true).unwrap();
    assert_eq!(rec.lists().len(), 1);
    assert!(manager.get("a").unwrap().keep_awake);
    assert!(rec.lists()[0].sessions[0].keep_awake);
    assert_eq!(manager.set_keep_awake("l", true).unwrap_err().to_string(), "Only a Claude tab can be kept awake.");
    assert!(manager.set_keep_awake("nope", true).is_err());
    manager.set_keep_awake("a", false).unwrap();
    assert_eq!(rec.lists().len(), 2);
    assert!(!manager.get("a").unwrap().keep_awake);
}

// ---------- with a real pty (cargo test -p multi-claude -- --ignored) ----------

fn shell_plan(cwd: &str, script: &str) -> SpawnPlan {
    let (program, args) = if cfg!(windows) {
        (std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()), vec!["/d".to_owned(), "/c".to_owned(), script.to_owned()])
    } else {
        ("/bin/sh".to_owned(), vec!["-c".to_owned(), script.to_owned()])
    };
    SpawnPlan {
        id: new_session_id(),
        profile_id: "work".into(),
        kind: SessionKind::Claude,
        cwd: cwd.into(),
        program,
        args,
        env: crate::claude_binary::process_env(),
        title: title_of(cwd),
        name: None,
        place: None,
        replaces: None,
        claude_session_id: None,
        request: request(cwd, SessionKind::Claude),
    }
}

fn long_running() -> &'static str {
    if cfg!(windows) {
        "ping -n 60 127.0.0.1 >nul"
    } else {
        "sleep 60"
    }
}

fn exits(manager: &SessionManager) -> mpsc::Receiver<SessionInfo> {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    manager.set_exit_handler(Box::new(move |info| {
        let _ = lock(&tx).send(info);
    }));
    rx
}

#[test]
#[ignore = "starts a process in a real pty"]
fn a_session_runs_its_program_and_ends() {
    let (rec, manager) = manager();
    let ended = exits(&manager);
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let mut plan = shell_plan(&dir.path().to_string_lossy(), "echo hi");
    plan.place = Some(Place { group_id: Some("g".into()), after_id: None });
    let info = manager.create(plan).unwrap();
    assert_eq!(info.status, SessionStatus::Running);

    let done = ended.recv_timeout(Duration::from_secs(20)).expect("the session should end");
    assert_eq!(done.id, info.id);
    assert_eq!(done.status, SessionStatus::Exited);
    assert_eq!(done.exit_code, Some(0));

    let attached = manager.attach(&info.id).unwrap();
    assert!(attached.replay.contains("hi"), "{:?}", attached.replay);
    // Windows: the pseudoconsole's cursor request was answered here and never reached the window
    assert!(!attached.replay.contains("\x1b[6n"));
    let data = lock(&rec.data).clone();
    assert!(data.windows(2).all(|w| w[0].seq < w[1].seq));
    assert_eq!(data.last().unwrap().seq, attached.seq);
    assert_eq!(attached.seq as usize, mc_core::jsstr::len16(&data.iter().map(|d| d.data.as_str()).collect::<String>()));

    let lists = rec.lists();
    assert_eq!(lists[0].sessions[0].place.as_ref().unwrap().group_id.as_deref(), Some("g"));
    assert_eq!(lists.last().unwrap().sessions[0].status, SessionStatus::Exited);
    assert_eq!(lists.last().unwrap().sessions[0].place, None);
    assert!(lock(&cleanups).iter().any(|(_, s)| *s == info.id));
}

#[test]
#[ignore = "starts a process in a real pty"]
fn a_session_takes_input() {
    let (_rec, manager) = manager();
    let ended = exits(&manager);
    let dir = temp_folder();
    let mut plan = shell_plan(&dir.path().to_string_lossy(), "");
    if cfg!(windows) {
        plan.args = vec!["/d".into(), "/q".into()];
    } else {
        plan.args = vec![];
    }
    let info = manager.create(plan).unwrap();
    manager.resize(&info.id, 100, 40);
    let enter = if cfg!(windows) { "\r" } else { "\n" };
    assert!(manager.write(&info.id, &format!("echo mark{}er42{enter}", if cfg!(windows) { "^" } else { "\\" })));
    assert!(manager.write(&info.id, &format!("exit{enter}")));
    ended.recv_timeout(Duration::from_secs(20)).expect("the shell should exit");
    let replay = manager.attach(&info.id).unwrap().replay;
    assert!(replay.contains("marker42"), "{replay:?}");
    assert!(!manager.write(&info.id, "late"));
}

#[test]
#[ignore = "starts a process in a real pty"]
fn ending_a_session_waits_for_its_process() {
    let (_rec, manager) = manager();
    let ended = exits(&manager);
    let dir = temp_folder();
    let info = manager.create(shell_plan(&dir.path().to_string_lossy(), long_running())).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let started = std::time::Instant::now();
    rt.block_on(manager.end(&info.id, 10_000));
    assert!(started.elapsed() < Duration::from_secs(8));
    let done = ended.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(done.status, SessionStatus::Exited);
    // the tab stays, as an ended session; telling it again does nothing
    assert_eq!(manager.get(&info.id).unwrap().status, SessionStatus::Exited);
    manager.kill(&info.id);
    rt.block_on(manager.end(&info.id, 10_000));
}

#[test]
#[ignore = "starts a process in a real pty"]
fn a_new_session_takes_over_a_tab() {
    let (rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let cwd = dir.path().to_string_lossy().into_owned();
    let first = manager.create(shell_plan(&cwd, long_running())).unwrap();
    let other = manager.create(shell_plan(&cwd, long_running())).unwrap();
    let next = manager.replace(&first.id, shell_plan(&cwd, "echo next")).unwrap();
    assert_eq!(manager.list().iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec![next.id.clone(), other.id.clone()]);
    let lists = rec.lists();
    let introduced = lists.last().unwrap();
    assert_eq!(introduced.sessions[0].replaces.as_deref(), Some(first.id.as_str()));
    assert!(lock(&cleanups).iter().any(|(_, s)| *s == first.id));
    // a tab that is gone cannot be taken over
    let err = manager.replace(&first.id, shell_plan(&cwd, "echo x")).unwrap_err();
    assert_eq!(err.to_string(), "Session not found");
    manager.close_many(&[next.id.as_str(), other.id.as_str()]);
    assert!(manager.list().is_empty());
    assert!(rec.lists().last().unwrap().sessions.iter().all(|s| s.replaces.is_none()));
}

#[test]
#[ignore = "starts a process in a real pty"]
fn a_program_that_cannot_start_is_cleaned_up() {
    let (rec, manager) = manager();
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let mut plan = shell_plan(&dir.path().to_string_lossy(), "echo");
    plan.program = if cfg!(windows) { "C:\\surely\\not\\here.exe".into() } else { "/surely/not/here".into() };
    let id = plan.id.clone();
    assert!(manager.create(plan).is_err());
    assert!(manager.list().is_empty());
    assert!(rec.lists().is_empty());
    assert_eq!(lock(&cleanups).as_slice(), &[("work".to_owned(), id)]);
}

/// Waits at most 20 s for the session to come to `status`.
fn wait_for_status(manager: &SessionManager, id: &str, status: SessionStatus) {
    let started = std::time::Instant::now();
    while manager.get(id).map(|s| s.status) != Some(status) && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(manager.get(id).map(|s| s.status), Some(status));
}

#[test]
#[ignore = "starts a process in a real pty"]
fn a_sleeping_session_wakes_under_its_own_id_with_its_output() {
    let (rec, manager) = manager();
    let ended = exits(&manager);
    let cleanups = record_cleanups(&manager);
    let dir = temp_folder();
    let cwd = dir.path().to_string_lossy().into_owned();
    let script = if cfg!(windows) { "echo first& ping -n 60 127.0.0.1 >nul" } else { "echo first; sleep 60" };
    let info = manager.create(shell_plan(&cwd, script)).unwrap();
    let started = std::time::Instant::now();
    while !manager.attach(&info.id).unwrap().replay.contains("first") && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.resize(&info.id, 100, 40);

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(manager.sleep(&info.id, 5_000).unwrap());
    wait_for_status(&manager, &info.id, SessionStatus::Sleeping);
    // asleep is not ended: nobody hears of an exit, and the output stays
    assert!(ended.try_recv().is_err());
    let asleep = manager.attach(&info.id).unwrap();
    assert!(asleep.replay.contains("first"), "{:?}", asleep.replay);
    assert!(!manager.write(&info.id, "x"));
    assert!(manager.saved_tabs()[0].1.asleep);

    // a new process under the same id; its output follows the old one
    let mut plan = shell_plan(&cwd, "echo second");
    plan.id = info.id.clone();
    let woken = manager.wake(plan).unwrap();
    assert_eq!((woken.id.as_str(), woken.status, woken.slept_at), (info.id.as_str(), SessionStatus::Running, None));
    assert_eq!(manager.list().len(), 1);
    // this one ends by itself: an ordinary end this time
    let done = ended.recv_timeout(Duration::from_secs(20)).expect("the woken session should end");
    assert_eq!((done.id.as_str(), done.status), (info.id.as_str(), SessionStatus::Exited));
    let attached = manager.attach(&info.id).unwrap();
    let (first, second) = (attached.replay.find("first"), attached.replay.find("second"));
    assert!(first.is_some() && second.is_some() && first < second, "{:?}", attached.replay);
    // one sequence through both processes, as the window counts it
    let data = lock(&rec.data).clone();
    assert!(data.windows(2).all(|w| w[0].seq < w[1].seq));
    assert_eq!(data.last().unwrap().seq, attached.seq);
    assert_eq!(attached.seq as usize, mc_core::jsstr::len16(&data.iter().map(|d| d.data.as_str()).collect::<String>()));
    // both processes let go of their files
    assert_eq!(lock(&cleanups).iter().filter(|(_, s)| *s == info.id).count(), 2);
}
