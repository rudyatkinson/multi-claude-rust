//! The `/usage` queue (`UsageProber` and `runUsageProbe` of `src/main/usage/probe.ts`): asks Claude
//! Code for each account's plan usage, one account at a time.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use mc_core::usage::{parse_usage_output, LocalClock, ProbeResult, PROBE_ARGS};
use tauri::async_runtime::{self, JoinHandle};
use tokio::sync::oneshot;

use super::{lock, now_ms, read};

const INTERVAL: Duration = Duration::from_secs(10 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
/// `execFile`'s default `maxBuffer`: more output than this is a failure.
const MAX_OUTPUT: usize = 1024 * 1024;

/// As much of an account as probing it and naming it in an alert needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeProfile {
    pub id: String,
    /// The account's name, for alerts.
    pub name: String,
    /// `CLAUDE_CONFIG_DIR` of the account; None for Main.
    pub config_dir: Option<String>,
    /// `account.loggedIn` of the last status check.
    pub logged_in: bool,
    /// A token account (`auth: 'token'`): `/usage` prints an API cost summary for it, not plan usage.
    pub token: bool,
}

/// What the usage service needs from the rest of the app. Set once with
/// [`super::UsageTracker::set_deps`]; until then nothing is probed and no alert is delivered.
pub trait ProbeDeps: Send + Sync + 'static {
    /// The claude binary, when one was found.
    fn claude_path(&self) -> Option<PathBuf>;
    /// All accounts, in the order of the account list.
    fn profiles(&self) -> Vec<ProbeProfile>;
    /// The complete environment to run claude with for this account (the child gets nothing else).
    fn env_for(&self, profile: &ProbeProfile) -> BTreeMap<String, String>;
}

type RunFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
/// Runs claude for `/usage`: (claude path, environment, working folder) to its stdout.
pub(crate) type ProbeRun = Arc<dyn Fn(PathBuf, BTreeMap<String, String>, PathBuf) -> RunFuture + Send + Sync>;
pub(crate) type Apply = Box<dyn Fn(&str, ProbeResult) + Send + Sync>;

#[derive(Default)]
struct Queue {
    /// In insertion order; the first entry is being probed while `draining`. An account that is
    /// already waiting is not queued twice: its new callers wait for the same answer.
    pending: Vec<(String, Vec<oneshot::Sender<bool>>)>,
    draining: bool,
    last_all: i64,
    stopped: bool,
}

pub(crate) struct Prober {
    inner: Arc<Inner>,
}

struct Inner {
    /// An empty folder to run in, so that no project settings are picked up.
    cwd: PathBuf,
    deps: RwLock<Option<Arc<dyn ProbeDeps>>>,
    clock: RwLock<Arc<dyn LocalClock>>,
    apply: Apply,
    run: ProbeRun,
    queue: std::sync::Mutex<Queue>,
    drain_task: std::sync::Mutex<Option<JoinHandle<()>>>,
    timer: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Prober {
    pub(crate) fn new(cwd: PathBuf, clock: Arc<dyn LocalClock>, apply: Apply, run: ProbeRun) -> Self {
        Prober {
            inner: Arc::new(Inner {
                cwd,
                deps: RwLock::new(None),
                clock: RwLock::new(clock),
                apply,
                run,
                queue: Default::default(),
                drain_task: Default::default(),
                timer: Default::default(),
            }),
        }
    }

    pub(crate) fn set_deps(&self, deps: Arc<dyn ProbeDeps>) {
        *self.inner.deps.write().unwrap_or_else(|e| e.into_inner()) = Some(deps);
    }

    pub(crate) fn deps(&self) -> Option<Arc<dyn ProbeDeps>> {
        read(&self.inner.deps).clone()
    }

    pub(crate) fn set_clock(&self, clock: Arc<dyn LocalClock>) {
        *self.inner.clock.write().unwrap_or_else(|e| e.into_inner()) = clock;
    }

    /// A first round now, then one every 10 minutes.
    pub(crate) fn start(&self) {
        self.refresh_all();
        let inner = Arc::downgrade(&self.inner);
        let timer = async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(INTERVAL).await;
                let Some(inner) = inner.upgrade() else { return };
                Prober { inner }.refresh_all();
            }
        });
        if let Some(old) = lock(&self.inner.timer).replace(timer) {
            old.abort();
        }
    }

    /// Answers every waiting request with false and probes nothing more.
    pub(crate) fn stop(&self) {
        let pending = {
            let mut queue = lock(&self.inner.queue);
            queue.stopped = true;
            queue.draining = false;
            std::mem::take(&mut queue.pending)
        };
        for (_, waiters) in pending {
            for waiter in waiters {
                let _ = waiter.send(false);
            }
        }
        for slot in [&self.inner.timer, &self.inner.drain_task] {
            if let Some(task) = lock(slot).take() {
                task.abort();
            }
        }
    }

    pub(crate) fn refresh_all(&self) {
        lock(&self.inner.queue).last_all = now_ms();
        let Some(deps) = self.deps() else { return };
        for profile in deps.profiles() {
            drop(self.refresh(&profile.id));
        }
    }

    pub(crate) fn refresh_all_if_older_than(&self, ms: i64) {
        let last_all = lock(&self.inner.queue).last_all;
        if now_ms() - last_all > ms {
            self.refresh_all();
        }
    }

    /// Queues the account at once and resolves once it has been probed: with true when that
    /// produced numbers, which are applied by then. Dropping the future keeps the account queued.
    pub(crate) fn refresh(&self, profile_id: &str) -> impl Future<Output = bool> + Send + 'static {
        let (tx, rx) = oneshot::channel();
        let start = {
            let mut queue = lock(&self.inner.queue);
            if queue.stopped {
                let _ = tx.send(false);
                false
            } else {
                match queue.pending.iter_mut().find(|(id, _)| id == profile_id) {
                    Some((_, waiters)) => waiters.push(tx),
                    None => queue.pending.push((profile_id.to_string(), vec![tx])),
                }
                !std::mem::replace(&mut queue.draining, true)
            }
        };
        if start {
            let task = async_runtime::spawn(drain(self.inner.clone()));
            *lock(&self.inner.drain_task) = Some(task);
        }
        async move { rx.await.unwrap_or(false) }
    }
}

async fn drain(inner: Arc<Inner>) {
    loop {
        let profile_id = {
            let mut queue = lock(&inner.queue);
            match queue.pending.first() {
                Some((id, _)) if !queue.stopped => id.clone(),
                _ => {
                    queue.draining = false;
                    return;
                }
            }
        };
        let fresh = probe(&inner, &profile_id).await;
        let waiters = {
            let mut queue = lock(&inner.queue);
            match queue.pending.iter().position(|(id, _)| *id == profile_id) {
                Some(i) => queue.pending.remove(i).1,
                None => Vec::new(),
            }
        };
        for waiter in waiters {
            let _ = waiter.send(fresh);
        }
    }
}

async fn probe(inner: &Inner, profile_id: &str) -> bool {
    let Some(deps) = read(&inner.deps).clone() else { return false };
    let profile = deps.profiles().into_iter().find(|p| p.id == profile_id);
    let claude = deps.claude_path();
    let (Some(claude), Some(profile)) = (claude, profile) else { return false };
    // with a token, /usage prints an API cost summary instead of plan usage
    if !profile.logged_in || profile.token {
        return false;
    }
    let env = deps.env_for(&profile);
    match (inner.run)(claude, env, inner.cwd.clone()).await {
        Ok(output) => {
            let clock = read(&inner.clock).clone();
            match parse_usage_output(&output, now_ms(), clock.as_ref()) {
                Some(result) => {
                    (inner.apply)(profile_id, result);
                    true
                }
                None => false,
            }
        }
        Err(e) => {
            // offline, rate limited or an unexpected output format: the last known numbers stay
            log::debug!("usage probe of {profile_id}: {e}");
            false
        }
    }
}

/// The real command: `claude -p /usage ...` in `cwd`, with a 45 s limit.
pub(crate) fn real_run() -> ProbeRun {
    Arc::new(|claude, env, cwd| Box::pin(run_usage_probe(claude, env, cwd)))
}

async fn run_usage_probe(claude: PathBuf, env: BTreeMap<String, String>, cwd: PathBuf) -> Result<String, String> {
    // the folder may have been removed since the app started
    let _ = std::fs::create_dir_all(&cwd);
    let (program, args) = spawn_spec(&claude, &PROBE_ARGS);
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .env_clear()
        .envs(&env)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("claude exited with {}", output.status));
    }
    if output.stdout.len() > MAX_OUTPUT {
        return Err("too much output".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// npm installs claude as a .cmd shim on Windows, which has to go through cmd.exe
/// (`spawnSpec` of `src/main/claudeBinary.ts`).
fn spawn_spec(claude: &Path, args: &[&str]) -> (OsString, Vec<OsString>) {
    let args = args.iter().map(OsString::from);
    let shim = cfg!(windows)
        && claude.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    if shim {
        let comspec = std::env::var_os("ComSpec").filter(|c| !c.is_empty()).unwrap_or_else(|| "cmd.exe".into());
        let mut all: Vec<OsString> = vec!["/d".into(), "/c".into(), claude.as_os_str().to_owned()];
        all.extend(args);
        return (comspec, all);
    }
    (claude.as_os_str().to_owned(), args.collect())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use mc_core::usage::FixedOffsetClock;

    use super::*;

    const ELIF: &str = "Current session: 2% used · resets Oct 7, 3:50pm (Europe/Istanbul)
Current week (all models): 0% used · resets Oct 13, 1pm (Europe/Istanbul)
Current week (Fable): 0% used · resets Oct 13, 1pm (Europe/Istanbul)
";

    struct Deps(Vec<ProbeProfile>);

    impl ProbeDeps for Deps {
        fn claude_path(&self) -> Option<PathBuf> {
            Some(PathBuf::from("claude"))
        }
        fn profiles(&self) -> Vec<ProbeProfile> {
            self.0.clone()
        }
        fn env_for(&self, _profile: &ProbeProfile) -> BTreeMap<String, String> {
            BTreeMap::new()
        }
    }

    fn account(id: &str, logged_in: bool) -> ProbeProfile {
        ProbeProfile { id: id.into(), name: id.into(), config_dir: None, logged_in, token: false }
    }

    struct Setup {
        prober: Prober,
        applied: Arc<Mutex<Vec<(String, ProbeResult)>>>,
        runs: Arc<AtomicUsize>,
    }

    /// `run` stands in for the command: it is handed the call's number and returns its output.
    fn setup<F, Fut>(run: F, profiles: Vec<ProbeProfile>) -> Setup
    where
        F: Fn(usize) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        let applied = Arc::new(Mutex::new(Vec::new()));
        let runs = Arc::new(AtomicUsize::new(0));
        let sink = applied.clone();
        let counter = runs.clone();
        let run = Arc::new(run);
        let prober = Prober::new(
            PathBuf::from("."),
            Arc::new(FixedOffsetClock::UTC),
            Box::new(move |id: &str, result: ProbeResult| sink.lock().unwrap().push((id.to_string(), result))),
            Arc::new(move |_, _, _| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let run = run.clone();
                Box::pin(async move { run(n).await }) as RunFuture
            }),
        );
        prober.set_deps(Arc::new(Deps(profiles)));
        Setup { prober, applied, runs }
    }

    fn two() -> Vec<ProbeProfile> {
        vec![account("a", true), account("b", true)]
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        async_runtime::block_on(f)
    }

    #[test]
    fn resolves_a_refresh_once_the_numbers_have_been_applied() {
        let s = setup(|_| async { Ok(ELIF.to_string()) }, two());
        assert!(block_on(s.prober.refresh("a")));
        let applied = s.applied.lock().unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].0, "a");
        assert_eq!(applied[0].1.five_hour.as_ref().map(|w| w.used_pct), Some(2.0));
    }

    #[test]
    fn probes_an_account_once_for_overlapping_requests_and_runs_accounts_one_at_a_time() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (a, p) = (active.clone(), peak.clone());
        let s = setup(
            move |_| {
                let (a, p) = (a.clone(), p.clone());
                async move {
                    let now = a.fetch_add(1, Ordering::SeqCst) + 1;
                    p.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    a.fetch_sub(1, Ordering::SeqCst);
                    Ok(ELIF.to_string())
                }
            },
            two(),
        );
        let (x, y, z) = (s.prober.refresh("a"), s.prober.refresh("a"), s.prober.refresh("b"));
        block_on(async { (x.await, y.await, z.await) });
        assert_eq!(s.runs.load(Ordering::SeqCst), 2);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
        let ids: Vec<String> = s.applied.lock().unwrap().iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn still_resolves_when_the_command_fails_or_prints_something_else() {
        let failing = setup(|_| async { Err("offline".to_string()) }, two());
        assert!(!block_on(failing.prober.refresh("a")));
        assert!(failing.applied.lock().unwrap().is_empty());

        let garbage = setup(|_| async { Ok("Please run /login".to_string()) }, two());
        assert!(!block_on(garbage.prober.refresh("a")));
        assert!(garbage.applied.lock().unwrap().is_empty());
    }

    #[test]
    fn does_not_run_the_command_for_an_account_that_cannot_be_asked() {
        let out = setup(|_| async { Ok(ELIF.to_string()) }, vec![account("a", false)]);
        assert!(!block_on(out.prober.refresh("a")));
        assert_eq!(out.runs.load(Ordering::SeqCst), 0);

        let token = setup(|_| async { Ok(ELIF.to_string()) }, vec![ProbeProfile { token: true, ..account("a", true) }]);
        assert!(!block_on(token.prober.refresh("a")));
        assert_eq!(token.runs.load(Ordering::SeqCst), 0);

        let unknown = setup(|_| async { Ok(ELIF.to_string()) }, two());
        assert!(!block_on(unknown.prober.refresh("nobody")));
        assert_eq!(unknown.runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn answers_the_requests_that_are_still_waiting_when_it_is_stopped() {
        let s = setup(|_| std::future::pending(), two());
        let waiting = s.prober.refresh("a");
        let queued = s.prober.refresh("b");
        s.prober.stop();
        assert!(!block_on(waiting));
        assert!(!block_on(queued));
        // and nothing more after that
        assert!(!block_on(s.prober.refresh("a")));
    }

    #[test]
    fn refreshes_every_account_and_only_when_the_last_round_is_old_enough() {
        let s = setup(|_| async { Ok(ELIF.to_string()) }, two());
        s.prober.refresh_all();
        // the queue is drained in order; waiting on b means a is done too
        assert!(block_on(s.prober.refresh("b")));
        assert_eq!(s.runs.load(Ordering::SeqCst), 2);
        s.prober.refresh_all_if_older_than(60_000);
        assert!(block_on(s.prober.refresh("a")));
        assert_eq!(s.runs.load(Ordering::SeqCst), 3);
    }

    /// The real runner against a stand-in for claude that prints a usage line, its arguments, its
    /// folder and one variable of its environment.
    #[test]
    fn runs_the_command_in_the_probe_folder_with_the_given_environment() {
        let dir = tempfile::Builder::new().prefix("mc probe ").tempdir().unwrap();
        let cwd = dir.path().join("probe");
        let script = if cfg!(windows) {
            let file = dir.path().join("claude.cmd");
            std::fs::write(&file, "@echo Current session: 5%% used\r\n@echo args=%*\r\n@echo cwd=%CD%\r\n@echo mark=%MC_MARK%\r\n").unwrap();
            file
        } else {
            let file = dir.path().join("claude");
            std::fs::write(&file, "#!/bin/sh\necho 'Current session: 5% used'\necho \"args=$*\"\necho \"cwd=$(pwd)\"\necho \"mark=$MC_MARK\"\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            file
        };
        let mut env: BTreeMap<String, String> = std::env::vars().collect();
        env.insert("MC_MARK".into(), "probe-env".into());
        let out = block_on(real_run()(script, env, cwd.clone())).unwrap();
        assert!(out.contains(&format!("args={}", PROBE_ARGS.join(" "))), "{out}");
        assert!(out.contains("mark=probe-env"), "{out}");
        let shown = out.lines().find_map(|l| l.trim().strip_prefix("cwd=")).unwrap();
        assert_eq!(std::fs::canonicalize(shown).unwrap(), std::fs::canonicalize(&cwd).unwrap());
        let result = parse_usage_output(&out, 0, &FixedOffsetClock::UTC).unwrap();
        assert_eq!(result.five_hour.map(|w| w.used_pct), Some(5.0));
        // a command that fails is an error, whatever it printed
        let missing = block_on(real_run()(dir.path().join("nothing-here.exe"), BTreeMap::new(), cwd));
        assert!(missing.is_err());
    }

    #[test]
    fn runs_npm_shims_through_cmd() {
        let (program, args) = spawn_spec(Path::new("claude.exe"), &["-p"]);
        assert_eq!(program, OsString::from("claude.exe"));
        assert_eq!(args, vec![OsString::from("-p")]);
        let (program, args) = spawn_spec(Path::new(r"C:\npm\claude.CMD"), &["-p"]);
        if cfg!(windows) {
            assert!(program.to_string_lossy().to_lowercase().ends_with("cmd.exe"));
            assert_eq!(args, ["/d", "/c", r"C:\npm\claude.CMD", "-p"].map(OsString::from).to_vec());
        } else {
            assert_eq!(program, OsString::from(r"C:\npm\claude.CMD"));
        }
    }
}
