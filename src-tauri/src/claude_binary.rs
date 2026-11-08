//! Finding the `claude` executable and running it (`src/main/claudeBinary.ts`).
//!
//! Everything here that runs a process is async (tokio) and must be called inside the Tauri
//! runtime (`tauri::async_runtime`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use mc_core::types::ClaudeBinaryInfo;

/// `claude --version` gets this long to answer.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// `where.exe` and the login shell get this long.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Whether `claude` is a batch file, whose arguments pass through cmd.exe and its quoting rules.
pub fn is_shim(claude_path: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let lower = claude_path.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

/// The program to start and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
}

/// `spawnSpec`: npm installs claude as a .cmd shim on Windows, which has to go through cmd.exe.
pub fn spawn_spec(claude_path: &str, args: &[String]) -> SpawnSpec {
    if is_shim(claude_path) {
        let comspec = std::env::var("ComSpec").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "cmd.exe".into());
        let mut full = vec!["/d".to_owned(), "/c".to_owned(), claude_path.to_owned()];
        full.extend(args.iter().cloned());
        return SpawnSpec { program: comspec, args: full };
    }
    SpawnSpec { program: claude_path.to_owned(), args: args.to_vec() }
}

/// The login shell's PATH, once [`import_login_shell_path`] has read it (macOS).
static LOGIN_PATH: OnceLock<String> = OnceLock::new();

/// The app's own environment, as a map of texts (variables that are not valid Unicode are left
/// out). On macOS the PATH of the user's login shell replaces the inherited one once
/// [`import_login_shell_path`] has run, as the Electron version replaced `process.env.PATH`.
pub fn process_env() -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    if let Some(path) = LOGIN_PATH.get() {
        env.insert("PATH".into(), path.clone());
    }
    env
}

/// A development build: the dev tooling's `NODE_ENV` and `NODE_OPTIONS` are kept out of sessions.
pub fn is_dev() -> bool {
    cfg!(debug_assertions)
}

/// What a finished process left: its output, and an error unless it exited 0 in time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// None when the process exited with 0 within its time. Worded like Node's `execFile` errors.
    pub error: Option<String>,
}

impl RunOutput {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// Runs a program without a window and collects its output, as `execFile` did. `env` replaces
/// the whole environment when given. A process that takes longer than `timeout` is killed.
pub async fn run(program: &str, args: &[String], env: Option<&BTreeMap<String, String>>, cwd: Option<&Path>, timeout: Duration) -> RunOutput {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(env) = env {
        cmd.env_clear().envs(env);
    }
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let command_line = if args.is_empty() { program.to_owned() } else { format!("{program} {}", args.join(" ")) };
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return RunOutput { error: Some(format!("spawn {program}: {e}")), ..RunOutput::default() },
    };
    // on a timeout the future is dropped, and with it the child, which kill_on_drop ends
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Err(_) => RunOutput { error: Some(format!("Command failed: {command_line} (no answer within {} s)", timeout.as_secs())), ..RunOutput::default() },
        Ok(Err(e)) => RunOutput { error: Some(format!("Command failed: {command_line}: {e}")), ..RunOutput::default() },
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let error = (!out.status.success()).then(|| format!("Command failed: {command_line}\n{stderr}").trim_end().to_owned());
            RunOutput { code: out.status.code(), stdout, stderr, error }
        }
    }
}

/// Runs claude (through cmd.exe for a shim) with the given arguments.
pub async fn run_claude(claude_path: &str, args: &[String], env: Option<&BTreeMap<String, String>>, cwd: Option<&Path>, timeout: Duration) -> RunOutput {
    let spec = spawn_spec(claude_path, args);
    run(&spec.program, &spec.args, env, cwd, timeout).await
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// A variable of `env`; on Windows its name in any case.
fn env_var<'a>(env: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    if cfg!(windows) {
        env.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    } else {
        env.get(name).map(String::as_str)
    }
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn path_text(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// The non-empty lines of a command's output, trimmed.
fn lines(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

/// Where claude may be, in the order it is tried: the path from the settings first.
pub async fn candidates(preferred: Option<&str>, env: &BTreeMap<String, String>) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    if let Some(p) = preferred.filter(|p| !p.is_empty()) {
        list.push(p.to_owned());
    }
    if cfg!(windows) {
        list.push(path_text(home_dir().join(".local").join("bin").join("claude.exe")));
        let found = run("where.exe", &strings(&["claude"]), Some(env), None, LOOKUP_TIMEOUT).await;
        if found.ok() {
            let found = lines(&found.stdout);
            let ends = |p: &str, exts: &[&str]| {
                let lower = p.to_ascii_lowercase();
                exts.iter().any(|e| lower.ends_with(e))
            };
            list.extend(found.iter().filter(|p| ends(p, &[".exe"])).cloned());
            list.extend(found.iter().filter(|p| ends(p, &[".cmd", ".bat"])).cloned());
        }
        if let Some(appdata) = env_var(env, "APPDATA").filter(|a| !a.is_empty()) {
            list.push(path_text(Path::new(appdata).join("npm").join("claude.cmd")));
        }
    } else {
        let home = home_dir();
        list.push(path_text(home.join(".local").join("bin").join("claude")));
        list.push(path_text(home.join(".claude").join("local").join("claude")));
        list.push("/opt/homebrew/bin/claude".into());
        list.push("/usr/local/bin/claude".into());
        // GUI apps on macOS do not inherit the shell PATH, so ask a login shell
        let shell = env_var(env, "SHELL").filter(|s| !s.is_empty()).unwrap_or("/bin/zsh").to_owned();
        let out = run(&shell, &strings(&["-ilc", "command -v claude"]), Some(env), None, LOOKUP_TIMEOUT).await;
        if out.ok() {
            if let Some(hit) = lines(&out.stdout).into_iter().rfind(|l| l.starts_with('/')) {
                list.push(hit);
            }
        }
    }
    // [...new Set(list)]
    let mut seen = std::collections::HashSet::new();
    list.retain(|p| seen.insert(p.clone()));
    list
}

async fn is_file(p: &str) -> bool {
    tokio::fs::metadata(p).await.map(|m| m.is_file()).unwrap_or(false)
}

/// `resolveClaude`: the first candidate that is a file and answers `--version`, with that
/// version (the first line of its answer); or why none was found.
pub async fn find_claude_with_env(settings_path: Option<&str>, env: &BTreeMap<String, String>) -> ClaudeBinaryInfo {
    let preferred = settings_path.filter(|p| !p.is_empty());
    let mut last_error = "claude was not found".to_owned();
    for candidate in candidates(preferred, env).await {
        if !is_file(&candidate).await {
            if Some(candidate.as_str()) == preferred {
                last_error = format!("File not found: {candidate}");
            }
            continue;
        }
        let out = run_claude(&candidate, &strings(&["--version"]), Some(env), None, VERSION_TIMEOUT).await;
        match out.error {
            None => {
                let version = out.stdout.trim().lines().next().unwrap_or_default().to_owned();
                return ClaudeBinaryInfo { path: Some(candidate), version: Some(version), error: None };
            }
            Some(e) => last_error = format!("{candidate}: {e}"),
        }
    }
    ClaudeBinaryInfo { path: None, version: None, error: Some(last_error) }
}

/// `resolveClaude(settings.claudePath, buildClaudeEnv(process.env, null, { dev }))`.
pub async fn find_claude(settings_path: Option<&str>) -> ClaudeBinaryInfo {
    let env = mc_core::env::build_claude_env(&process_env(), None, None, is_dev());
    find_claude_with_env(settings_path, &env).await
}

/// macOS: the PATH of the user's login shell, so that tools started from a session find the
/// user's binaries. None elsewhere, and when the shell gives no answer.
pub async fn login_shell_path() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let out = run(&shell, &strings(&["-ilc", "echo \"__MC_PATH__$PATH\""]), None, None, LOOKUP_TIMEOUT).await;
    if !out.ok() {
        return None;
    }
    let line = out.stdout.lines().find(|l| l.starts_with("__MC_PATH__"))?;
    let path = line["__MC_PATH__".len()..].trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// `importLoginShellPath`: on macOS, reads the login shell's PATH once and uses it in
/// [`process_env`] from then on. Call it at startup, before claude is looked for.
pub async fn import_login_shell_path() {
    if let Some(path) = login_shell_path().await {
        let _ = LOGIN_PATH.set(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shims_are_batch_files_on_windows_only() {
        assert_eq!(is_shim("C:\\npm\\claude.cmd"), cfg!(windows));
        assert_eq!(is_shim("C:\\npm\\CLAUDE.BAT"), cfg!(windows));
        assert!(!is_shim("C:\\bin\\claude.exe"));
        assert!(!is_shim("/usr/local/bin/claude"));
        assert!(!is_shim("claude.cmd.exe"));
    }

    #[test]
    fn a_shim_goes_through_cmd() {
        let args = strings(&["--version"]);
        let spec = spawn_spec("C:\\npm\\claude.cmd", &args);
        if cfg!(windows) {
            assert!(spec.program.to_ascii_lowercase().ends_with("cmd.exe"));
            assert_eq!(spec.args, strings(&["/d", "/c", "C:\\npm\\claude.cmd", "--version"]));
        } else {
            assert_eq!(spec, SpawnSpec { program: "C:\\npm\\claude.cmd".into(), args: args.clone() });
        }
        assert_eq!(spawn_spec("/bin/claude", &args), SpawnSpec { program: "/bin/claude".into(), args: strings(&["--version"]) });
    }

    #[test]
    fn output_lines_are_trimmed_and_empty_ones_dropped() {
        assert_eq!(lines("C:\\a\\claude.exe\r\n\r\n  C:\\b\\claude.cmd  \r\n"), strings(&["C:\\a\\claude.exe", "C:\\b\\claude.cmd"]));
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn runs_a_program_and_reports_failures_like_exec_file() {
        let rt = runtime();
        let (ok, fail) = if cfg!(windows) {
            let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
            (
                rt.block_on(run(&comspec, &strings(&["/d", "/c", "echo 2.1.0 (Claude Code)"]), None, None, Duration::from_secs(10))),
                rt.block_on(run(&comspec, &strings(&["/d", "/c", "exit 3"]), None, None, Duration::from_secs(10))),
            )
        } else {
            (
                rt.block_on(run("/bin/sh", &strings(&["-c", "echo '2.1.0 (Claude Code)'"]), None, None, Duration::from_secs(10))),
                rt.block_on(run("/bin/sh", &strings(&["-c", "exit 3"]), None, None, Duration::from_secs(10))),
            )
        };
        assert!(ok.ok(), "{:?}", ok.error);
        assert_eq!(ok.stdout.trim(), "2.1.0 (Claude Code)");
        assert_eq!(fail.code, Some(3));
        assert!(fail.error.as_deref().unwrap().starts_with("Command failed: "));
        let missing = rt.block_on(run("surely-not-a-program-mc", &[], None, None, Duration::from_secs(10)));
        assert!(missing.error.as_deref().unwrap().starts_with("spawn surely-not-a-program-mc"));
    }

    #[test]
    fn a_process_that_takes_too_long_is_given_up() {
        let rt = runtime();
        let started = std::time::Instant::now();
        // the program itself waits: a grandchild would keep the output open after the kill
        let out = if cfg!(windows) {
            let ping = format!("{}\\System32\\PING.EXE", std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()));
            rt.block_on(run(&ping, &strings(&["-n", "20", "127.0.0.1"]), None, None, Duration::from_millis(300)))
        } else {
            rt.block_on(run("/bin/sh", &strings(&["-c", "sleep 20"]), None, None, Duration::from_millis(300)))
        };
        assert!(out.error.as_deref().unwrap().contains("no answer within"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn the_settings_path_comes_first_and_nothing_twice() {
        let rt = runtime();
        // an environment without PATH or SHELL: only the fixed places
        let env = BTreeMap::from([("APPDATA".to_owned(), "C:\\Users\\u\\AppData\\Roaming".to_owned())]);
        let local = path_text(home_dir().join(".local").join("bin").join(if cfg!(windows) { "claude.exe" } else { "claude" }));
        let list = rt.block_on(candidates(Some(&local), &env));
        assert_eq!(list[0], local);
        assert_eq!(list.iter().filter(|p| **p == local).count(), 1);
        if cfg!(windows) {
            assert_eq!(list.last().map(String::as_str), Some("C:\\Users\\u\\AppData\\Roaming\\npm\\claude.cmd"));
        } else {
            assert!(list.contains(&"/opt/homebrew/bin/claude".to_owned()));
        }
        // an empty settings path is no path
        let without = rt.block_on(candidates(Some(""), &env));
        assert_eq!(without[0], local);
    }
}
