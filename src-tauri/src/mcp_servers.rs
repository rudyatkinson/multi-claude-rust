//! Copying MCP servers between accounts through `claude mcp` (`src/main/mcpServers.ts`).
//!
//! A server definition may hold secrets (headers, env). It never leaves the backend: the window
//! sees names, transports and whether the other account has the same definition.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use mc_core::error::{AppError, AppResult};
use mc_core::jsstr::cut16;
use mc_core::settings_merge::{stable_stringify, to_js_json};
use mc_core::types::{McpCopyResult, McpServerRow, McpServerStatus};
use mc_core::validate::is_mcp_server_name;
use serde_json::Value;

use crate::launchers::is_cmd_shim;

/// Server definitions by name, as Claude Code stores them.
pub type McpServers = BTreeMap<String, Value>;

/// A boxed future that can cross threads.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long one `claude mcp` call may take.
pub const MCP_TIMEOUT: Duration = Duration::from_secs(20);

/// `claudeJsonFile`: `.claude.json` in the home folder for Main (`config_dir` None), inside the
/// config dir otherwise.
pub fn claude_json_file(config_dir: Option<&str>, home: &Path) -> PathBuf {
    match config_dir {
        Some(dir) => Path::new(dir).join(".claude.json"),
        None => home.join(".claude.json"),
    }
}

/// The user-level servers in a `.claude.json` file; none when the file is missing or unreadable.
/// Only names Claude Code accepts and object definitions count.
pub fn read_user_servers_file(file: &Path) -> McpServers {
    let Some(json) = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return McpServers::new();
    };
    let Some(Value::Object(servers)) = json.get("mcpServers") else {
        return McpServers::new();
    };
    servers.iter().filter(|(name, def)| is_mcp_server_name(name) && def.is_object()).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The user-level servers of an account whose `.claude.json` is in `dir` (the home folder for
/// Main, the config dir otherwise).
pub fn read_user_servers(dir: &Path) -> McpServers {
    read_user_servers_file(&dir.join(".claude.json"))
}

fn transport_of(def: &Value) -> String {
    // a server without a type is a local command
    match def.get("type") {
        Some(Value::String(t)) if !t.is_empty() => t.clone(),
        _ => "stdio".to_owned(),
    }
}

/// A definition with the default spelled out, so that `{command}` and `{type: "stdio", command}`
/// compare equal.
fn normalized(def: &Value) -> String {
    let transport = transport_of(def);
    let mut def = def.clone();
    if let Value::Object(obj) = &mut def {
        obj.insert("type".to_owned(), Value::String(transport));
    }
    stable_stringify(&def)
}

/// `a.localeCompare(b)` for server names (`[A-Za-z0-9_-]`), as the default collation orders them:
/// `_` before `-` before digits before letters, letters without regard to case first, then lower
/// case before upper case.
fn locale_compare(a: &str, b: &str) -> Ordering {
    fn primary(c: char) -> u32 {
        match c {
            '_' => 0,
            '-' => 1,
            '0'..='9' => 2 + c as u32 - '0' as u32,
            'a'..='z' => 12 + c as u32 - 'a' as u32,
            'A'..='Z' => 12 + c as u32 - 'A' as u32,
            other => 100 + other as u32,
        }
    }
    let primary_order = a.chars().map(primary).cmp(b.chars().map(primary));
    primary_order.then_with(|| a.chars().map(|c| c.is_ascii_uppercase()).cmp(b.chars().map(|c| c.is_ascii_uppercase()))).then_with(|| a.cmp(b))
}

/// What copying the main account's servers would do to the target account, without any definition.
pub fn diff_mcp_servers(main: &McpServers, target: &McpServers) -> Vec<McpServerRow> {
    let mut names: Vec<&String> = main.keys().collect();
    names.sort_by(|a, b| locale_compare(a, b));
    names
        .into_iter()
        .map(|name| {
            let def = &main[name];
            let status = match target.get(name) {
                None => McpServerStatus::Missing,
                Some(other) if normalized(def) == normalized(other) => McpServerStatus::Same,
                Some(_) => McpServerStatus::Different,
            };
            McpServerRow { name: name.clone(), transport: transport_of(def), status }
        })
        .collect()
}

/// The outcome of one `claude mcp` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliResult {
    pub ok: bool,
    /// The first line of what the command printed on failure.
    pub error: Option<String>,
}

impl CliResult {
    pub fn ok() -> Self {
        CliResult { ok: true, error: None }
    }
    pub fn failed(error: impl Into<String>) -> Self {
        CliResult { ok: false, error: Some(error.into()) }
    }
}

/// Runs `claude mcp ...` for the target account.
pub trait McpRunner: Send + Sync {
    fn run(&self, args: Vec<String>) -> BoxFuture<'_, CliResult>;
}

fn add_args(name: &str, def: &Value) -> Vec<String> {
    ["mcp", "add-json", "--scope", "user", "--", name].iter().map(|s| (*s).to_owned()).chain([to_js_json(def)]).collect()
}

fn remove_args(name: &str) -> Vec<String> {
    ["mcp", "remove", "--scope", "user", "--", name].iter().map(|s| (*s).to_owned()).collect()
}

/// Copies the named servers from `main` into the target account through Claude Code's own
/// command, one after the other. A server that differs is replaced; if the new definition is
/// refused, the old one is put back.
pub async fn copy_mcp_servers(names: &[String], main: &McpServers, target: &McpServers, runner: &dyn McpRunner) -> Vec<McpCopyResult> {
    let mut results = Vec::with_capacity(names.len());
    for name in names {
        let Some(def) = main.get(name) else {
            results.push(McpCopyResult { name: name.clone(), ok: false, error: Some("Not a server of the main account".to_owned()) });
            continue;
        };
        let old = target.get(name);
        if old.is_some() {
            let removed = runner.run(remove_args(name)).await;
            if !removed.ok {
                results.push(McpCopyResult { name: name.clone(), ok: false, error: removed.error });
                continue;
            }
        }
        let added = runner.run(add_args(name, def)).await;
        if !added.ok {
            if let Some(old) = old {
                runner.run(add_args(name, old)).await;
            }
        }
        results.push(McpCopyResult { name: name.clone(), ok: added.ok, error: if added.ok { None } else { added.error } });
    }
    results
}

/// `mcpRunner`: starts the claude executable directly, with the account's environment, in `cwd`
/// (the empty probe folder, so no project settings load), for at most 20 s.
pub struct ClaudeMcpRunner {
    claude: PathBuf,
    env: BTreeMap<String, String>,
    cwd: PathBuf,
}

impl ClaudeMcpRunner {
    /// A `.cmd` shim would send the JSON through cmd.exe, which reads `&`, `|` and `%` in it as its
    /// own syntax, so those installs are refused.
    pub fn new(claude_path: &str, env: BTreeMap<String, String>, cwd: PathBuf) -> AppResult<Self> {
        if is_cmd_shim(claude_path) {
            return Err(AppError::refused("Copying MCP servers needs the native Claude Code install (claude.exe)."));
        }
        Ok(ClaudeMcpRunner { claude: PathBuf::from(claude_path), env, cwd })
    }

    async fn exec(&self, args: Vec<String>) -> CliResult {
        let mut cmd = tokio::process::Command::new(&self.claude);
        cmd.args(&args)
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let stderr = match cmd.spawn() {
            Err(_) => String::new(),
            Ok(child) => match tokio::time::timeout(MCP_TIMEOUT, child.wait_with_output()).await {
                Ok(Ok(out)) if out.status.success() => return CliResult::ok(),
                Ok(Ok(out)) => String::from_utf8_lossy(&out.stderr).into_owned(),
                // timed out (the child is killed as it is dropped) or could not be waited for
                _ => String::new(),
            },
        };
        // not the error of the call: it would repeat the command line, and with it the definition
        CliResult::failed(first_line(&stderr).unwrap_or_else(|| "Claude Code refused the server".to_owned()))
    }
}

/// The first line of a command's error output, cut to 200 UTF-16 units; None when it is empty.
fn first_line(stderr: &str) -> Option<String> {
    let line = stderr.trim().split('\n').next().unwrap_or("").trim_end_matches('\r');
    let line = cut16(line, 200);
    (!line.is_empty()).then(|| line.to_owned())
}

impl McpRunner for ClaudeMcpRunner {
    fn run(&self, args: Vec<String>) -> BoxFuture<'_, CliResult> {
        Box::pin(self.exec(args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn servers(v: Value) -> McpServers {
        serde_json::from_value(v).unwrap()
    }

    // ---- readUserServers ----

    #[test]
    fn reads_the_user_level_servers_and_nothing_else_of_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".claude.json"),
            json!({
                "oauthAccount": { "emailAddress": "someone@example.com" },
                "mcpServers": { "unity": { "command": "node", "args": ["server.js"] }, "bad name!": { "command": "x" }, "broken": "not an object" },
                "projects": { "C:/code": { "mcpServers": { "local": { "command": "y" } } } }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(read_user_servers(tmp.path()), servers(json!({ "unity": { "command": "node", "args": ["server.js"] } })));
    }

    #[test]
    fn finds_none_in_a_missing_or_damaged_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_user_servers_file(&tmp.path().join("missing.json")).is_empty());
        let file = tmp.path().join(".claude.json");
        std::fs::write(&file, r#"{"mcpServers": [1, 2]}"#).unwrap();
        assert!(read_user_servers_file(&file).is_empty());
        std::fs::write(&file, "{ not json").unwrap();
        assert!(read_user_servers_file(&file).is_empty());
    }

    #[test]
    fn looks_next_to_the_home_folder_for_main_and_inside_the_config_dir_for_other_accounts() {
        let home = Path::new("/home/me");
        assert_eq!(claude_json_file(None, home), home.join(".claude.json"));
        assert_eq!(claude_json_file(Some("/p/elif"), home), Path::new("/p/elif").join(".claude.json"));
    }

    // ---- diffMcpServers ----

    fn main_servers() -> McpServers {
        servers(json!({
            "sentry": { "type": "http", "url": "https://mcp.example.com/mcp", "headers": { "Authorization": "Bearer secret" } },
            "unity": { "command": "node", "args": ["a.js"], "env": { "KEY": "secret" } },
            "files": { "type": "stdio", "command": "files" }
        }))
    }

    fn row(name: &str, transport: &str, status: McpServerStatus) -> McpServerRow {
        McpServerRow { name: name.into(), transport: transport.into(), status }
    }

    #[test]
    fn tells_what_a_copy_would_change_by_name_and_kind_only() {
        let target = servers(json!({
            // the same server with the default type spelled out
            "unity": { "type": "stdio", "env": { "KEY": "secret" }, "args": ["a.js"], "command": "node" },
            "files": { "type": "stdio", "command": "files", "args": ["--read-only"] },
            "own": { "command": "only-here" }
        }));
        let rows = diff_mcp_servers(&main_servers(), &target);
        assert_eq!(
            rows,
            vec![
                row("files", "stdio", McpServerStatus::Different),
                row("sentry", "http", McpServerStatus::Missing),
                row("unity", "stdio", McpServerStatus::Same),
            ]
        );
        assert!(!serde_json::to_string(&rows).unwrap().contains("secret"));
    }

    #[test]
    fn lists_nothing_when_main_has_no_servers() {
        assert!(diff_mcp_servers(&McpServers::new(), &servers(json!({ "own": { "command": "x" } }))).is_empty());
    }

    #[test]
    fn sorts_names_as_locale_compare_does() {
        let main = servers(json!({ "b": {}, "A": {}, "a": {}, "a-b": {}, "a_b": {}, "a1": {}, "B": {} }));
        let names: Vec<String> = diff_mcp_servers(&main, &McpServers::new()).into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["a", "A", "a_b", "a-b", "a1", "b", "B"]);
    }

    // ---- copyMcpServers ----

    type FailWhen = Box<dyn Fn(&[String]) -> bool + Send + Sync>;

    /// A stand-in for the claude command that records what it was asked and fails where told to.
    struct Recorder {
        calls: Mutex<Vec<Vec<String>>>,
        fail: FailWhen,
    }

    impl Recorder {
        fn new(fail: impl Fn(&[String]) -> bool + Send + Sync + 'static) -> Self {
            Recorder { calls: Mutex::new(Vec::new()), fail: Box::new(fail) }
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl McpRunner for Recorder {
        fn run(&self, args: Vec<String>) -> BoxFuture<'_, CliResult> {
            let failed = (self.fail)(&args);
            self.calls.lock().unwrap().push(args);
            Box::pin(async move { if failed { CliResult::failed("refused") } else { CliResult::ok() } })
        }
    }

    fn copy_main() -> McpServers {
        servers(json!({ "a": { "command": "new-a" }, "b": { "type": "http", "url": "https://b.example.com/?x=1&y=2" } }))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn result(name: &str, ok: bool, error: Option<&str>) -> McpCopyResult {
        McpCopyResult { name: name.into(), ok, error: error.map(str::to_owned) }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn adds_a_missing_server_with_its_exact_definition() {
        let cli = Recorder::new(|_| false);
        assert_eq!(block_on(copy_mcp_servers(&names(&["b"]), &copy_main(), &McpServers::new(), &cli)), vec![result("b", true, None)]);
        assert_eq!(cli.calls(), vec![names(&["mcp", "add-json", "--scope", "user", "--", "b", r#"{"type":"http","url":"https://b.example.com/?x=1&y=2"}"#])]);
    }

    #[test]
    fn replaces_a_server_that_differs_remove_then_add() {
        let cli = Recorder::new(|_| false);
        block_on(copy_mcp_servers(&names(&["a"]), &copy_main(), &servers(json!({ "a": { "command": "old-a" } })), &cli));
        let calls = cli.calls();
        assert_eq!(calls.iter().map(|c| format!("{} {} {}", c[0], c[1], c[5])).collect::<Vec<_>>(), vec!["mcp remove a", "mcp add-json a"]);
        assert_eq!(calls[1][6], r#"{"command":"new-a"}"#);
    }

    #[test]
    fn puts_the_old_definition_back_when_the_new_one_is_refused() {
        let cli = Recorder::new(|args| args[1] == "add-json" && args[6].contains("new-a"));
        let results = block_on(copy_mcp_servers(&names(&["a", "b"]), &copy_main(), &servers(json!({ "a": { "command": "old-a" } })), &cli));
        assert_eq!(results, vec![result("a", false, Some("refused")), result("b", true, None)]);
        let calls: Vec<String> = cli.calls().iter().map(|c| format!("{} {} {}", c[1], c[5], c.get(6).map_or("", String::as_str)).trim().to_owned()).collect();
        assert_eq!(
            calls,
            vec![
                "remove a",
                r#"add-json a {"command":"new-a"}"#,
                r#"add-json a {"command":"old-a"}"#,
                r#"add-json b {"type":"http","url":"https://b.example.com/?x=1&y=2"}"#,
            ]
        );
    }

    #[test]
    fn leaves_a_server_alone_when_it_cannot_be_removed_and_skips_unknown_names() {
        let cli = Recorder::new(|args| args[1] == "remove");
        let results = block_on(copy_mcp_servers(&names(&["a", "nope"]), &copy_main(), &servers(json!({ "a": { "command": "old-a" } })), &cli));
        assert_eq!(results, vec![result("a", false, Some("refused")), result("nope", false, Some("Not a server of the main account"))]);
        assert_eq!(cli.calls().len(), 1);
    }

    // ---- mcpRunner ----

    #[test]
    fn refuses_a_command_shim_which_would_pass_the_json_through_cmd_exe() {
        let cwd = PathBuf::from(".");
        let err = ClaudeMcpRunner::new(r"C:\Users\me\AppData\Roaming\npm\claude.cmd", BTreeMap::new(), cwd.clone()).err().unwrap();
        assert!(err.to_string().contains("native Claude Code install"));
        assert!(ClaudeMcpRunner::new(r"C:\tools\claude.BAT", BTreeMap::new(), cwd.clone()).is_err());
        assert!(ClaudeMcpRunner::new(r"C:\Users\me\.local\bin\claude.exe", BTreeMap::new(), cwd).is_ok());
    }

    #[test]
    fn a_missing_executable_reports_a_plain_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        let runner = ClaudeMcpRunner::new(&tmp.path().join("no-such-claude.exe").to_string_lossy(), BTreeMap::new(), tmp.path().to_path_buf()).unwrap();
        assert_eq!(block_on(runner.run(remove_args("x"))), CliResult::failed("Claude Code refused the server"));
    }

    #[test]
    fn the_error_is_the_first_line_of_stderr() {
        assert_eq!(first_line("\n  Invalid JSON: x\r\nmore\n").as_deref(), Some("Invalid JSON: x"));
        assert_eq!(first_line("   "), None);
        assert_eq!(first_line(&"e".repeat(300)).unwrap().len(), 200);
    }
}
