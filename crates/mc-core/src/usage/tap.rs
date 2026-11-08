//! The status line tap (`src/main/usage/statuslineTap.ts`, the pure part): a status line command
//! that saves the JSON Claude Code sends on stdin to a file of the app, then hands the same JSON to
//! the user's own status line. Finding Git Bash and reading the settings files is the backend's.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::js_trim;

/// The status files in the run folder: `<profile id>.<session id>.status.json`.
pub const STATUS_SUFFIX: &str = ".status.json";
const SETTINGS_SUFFIX: &str = ".settings.json";

/// The name of the file the tap of a session writes, in the run folder.
pub fn status_file_name(profile_id: &str, session_id: &str) -> String {
    format!("{profile_id}.{session_id}{STATUS_SUFFIX}")
}

/// The name of the settings file a session is started with, in the run folder.
pub fn settings_file_name(profile_id: &str, session_id: &str) -> String {
    format!("{profile_id}.{session_id}{SETTINGS_SUFFIX}")
}

/// (profile id, session id) of a status file name, as `filename.split('.')` took them apart: the
/// first two dot-separated parts (the session id is empty when there is no second part). None for
/// a name that is not a status file.
pub fn parse_status_file_name(name: &str) -> Option<(&str, &str)> {
    if !name.ends_with(STATUS_SUFFIX) {
        return None;
    }
    let mut parts = name.split('.');
    let profile_id = parts.next()?;
    Some((profile_id, parts.next().unwrap_or("")))
}

/// Quotes a value for a POSIX shell.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Git Bash eats unquoted backslashes, so paths in status line commands use forward slashes.
pub fn posix_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// A status line command that saves the JSON Claude Code sends on stdin to `status_file`
/// (written atomically, since a run can be cancelled mid-way) and then hands the same JSON
/// to the user's own status line command `inner`, or prints `label` when they have none.
pub fn tap_command(status_file: &str, inner: Option<&str>, label: &str) -> String {
    let file = sh_quote(&posix_path(status_file));
    let tmp = sh_quote(&format!("{}.tmp", posix_path(status_file)));
    let capture = format!("mc_in=$(cat); {{ printf '%s' \"$mc_in\" > {tmp} && mv -f {tmp} {file}; }} 2>/dev/null");
    match inner.filter(|cmd| !js_trim(cmd).is_empty()) {
        Some(cmd) => format!("{capture}; printf '%s' \"$mc_in\" | {{ {cmd}\n}}"),
        None => format!("{capture}; printf '%s' {}", sh_quote(label)),
    }
}

/// `{ statusLine: { ...inner, type: 'command', command: <tap> } }`: the settings that put the tap
/// in front of the status line `inner`, keeping its other fields (padding, ...).
pub fn tap_settings(status_file: &str, inner: Option<&Map<String, Value>>, label: &str) -> Value {
    let inner_command = inner.and_then(|line| line.get("command")).and_then(Value::as_str);
    let mut line = inner.cloned().unwrap_or_default();
    line.insert("type".into(), Value::String("command".into()));
    line.insert("command".into(), Value::String(tap_command(status_file, inner_command, label)));
    let mut settings = Map::new();
    settings.insert("statusLine".into(), Value::Object(line));
    Value::Object(settings)
}

/// The settings files that can define a session's status line, by Claude Code's precedence.
pub fn status_line_sources(cwd: &Path, config_dir: &Path) -> [PathBuf; 3] {
    [cwd.join(".claude").join("settings.local.json"), cwd.join(".claude").join("settings.json"), config_dir.join("settings.json")]
}

/// The `statusLine` of a settings file's text, when it has a command.
pub fn status_line_of(settings_text: &str) -> Option<Map<String, Value>> {
    let json: Value = serde_json::from_str(settings_text).ok()?;
    let line = json.get("statusLine")?.as_object()?;
    let command = line.get("command")?.as_str()?;
    (!js_trim(command).is_empty()).then(|| line.clone())
}

/// The status line the session would use without the tap. `read` returns a file's text, or None
/// when it is missing or unreadable.
pub fn find_inner_status_line(cwd: &Path, config_dir: &Path, mut read: impl FnMut(&Path) -> Option<String>) -> Option<Map<String, Value>> {
    status_line_sources(cwd, config_dir).iter().find_map(|file| read(file).as_deref().and_then(status_line_of))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn names_and_reads_status_files() {
        assert_eq!(status_file_name("p1", "s1"), "p1.s1.status.json");
        assert_eq!(settings_file_name("p1", "s1"), "p1.s1.settings.json");
        assert_eq!(parse_status_file_name("p1.s1.status.json"), Some(("p1", "s1")));
        assert_eq!(parse_status_file_name("p1.s1.status.json.tmp"), None);
        assert_eq!(parse_status_file_name("p1.s1.settings.json"), None);
    }

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(sh_quote("Ali's"), r"'Ali'\''s'");
        assert_eq!(posix_path(r"C:\run\a.json"), "C:/run/a.json");
    }

    #[test]
    fn builds_the_tap_command_exactly() {
        assert_eq!(
            tap_command(r"C:\run\p.s.status.json", Some("my-line"), "Work"),
            "mc_in=$(cat); { printf '%s' \"$mc_in\" > 'C:/run/p.s.status.json.tmp' && mv -f 'C:/run/p.s.status.json.tmp' 'C:/run/p.s.status.json'; } 2>/dev/null; printf '%s' \"$mc_in\" | { my-line\n}"
        );
        assert_eq!(
            tap_command("/r/x.status.json", Some("  "), "Ali's"),
            "mc_in=$(cat); { printf '%s' \"$mc_in\" > '/r/x.status.json.tmp' && mv -f '/r/x.status.json.tmp' '/r/x.status.json'; } 2>/dev/null; printf '%s' 'Ali'\\''s'"
        );
    }

    #[test]
    fn keeps_the_other_fields_of_the_wrapped_status_line() {
        let inner: Map<String, Value> = serde_json::from_str(r#"{"type":"command","command":"my-line","padding":2}"#).unwrap();
        let settings = tap_settings(r"C:\run\a.status.json", Some(&inner), "Work");
        let line = &settings["statusLine"];
        assert_eq!(line["padding"], 2);
        assert_eq!(line["type"], "command");
        let command = line["command"].as_str().unwrap();
        assert!(command.contains("'C:/run/a.status.json'"));
        assert!(command.contains("| { my-line\n}"));
        // without a status line of its own the session shows the account's name
        let plain = tap_settings("/r/a.status.json", None, "Work");
        assert_eq!(plain["statusLine"]["type"], "command");
        assert!(plain["statusLine"]["command"].as_str().unwrap().ends_with("printf '%s' 'Work'"));
    }

    #[test]
    fn finds_the_status_line_by_settings_precedence() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("project");
        let config_dir = root.path().join("config");
        fs::create_dir_all(cwd.join(".claude")).unwrap();
        fs::create_dir_all(&config_dir).unwrap();
        let find = || find_inner_status_line(&cwd, &config_dir, |p| fs::read_to_string(p).ok());
        let command = |line: Option<Map<String, Value>>| line.and_then(|l| l.get("command").and_then(Value::as_str).map(str::to_string));
        assert_eq!(find(), None);
        fs::write(config_dir.join("settings.json"), r#"{"statusLine":{"type":"command","command":"user"}}"#).unwrap();
        assert_eq!(command(find()).as_deref(), Some("user"));
        fs::write(cwd.join(".claude").join("settings.json"), r#"{"statusLine":{"type":"command","command":"project"}}"#).unwrap();
        assert_eq!(command(find()).as_deref(), Some("project"));
        // a status line without a command, or a file that is not JSON, does not count
        fs::write(cwd.join(".claude").join("settings.local.json"), r#"{"statusLine":{"type":"command","command":" "}}"#).unwrap();
        assert_eq!(command(find()).as_deref(), Some("project"));
        fs::write(cwd.join(".claude").join("settings.local.json"), "{").unwrap();
        assert_eq!(command(find()).as_deref(), Some("project"));
        fs::write(cwd.join(".claude").join("settings.local.json"), r#"{"statusLine":{"command":"local"}}"#).unwrap();
        assert_eq!(command(find()).as_deref(), Some("local"));
    }

    /// A POSIX shell to run the tap with: Git Bash on Windows, `/bin/sh` elsewhere.
    fn shell() -> Option<PathBuf> {
        if cfg!(windows) {
            [r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files (x86)\Git\bin\bash.exe"].into_iter().map(PathBuf::from).find(|p| p.exists())
        } else {
            Some(PathBuf::from("/bin/sh"))
        }
    }

    fn run(shell: &Path, command: &str, input: &str) -> String {
        let mut child =
            Command::new(shell).arg("-c").arg(command).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("the shell starts");
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn saves_stdin_to_the_status_file_and_still_feeds_the_wrapped_command() {
        let Some(shell) = shell() else {
            eprintln!("skipped: no Git Bash");
            return;
        };
        let dir = tempfile::Builder::new().prefix("mc tap o'brien-").tempdir().unwrap();
        let status_file = dir.path().join("p.s.status.json");
        let status = status_file.to_string_lossy();
        let payload = r#"{"rate_limits":{"five_hour":{"used_percentage":7,"resets_at":1738425600}},"note":"a \"quoted\" $HOME \\ value"}"#;

        let wrapped = run(&shell, &tap_command(&status, Some("tr a-z A-Z | head -c 12"), "Work"), payload);
        assert_eq!(wrapped, payload.to_uppercase()[..12]);
        assert_eq!(fs::read_to_string(&status_file).unwrap(), payload);
        assert!(!dir.path().join("p.s.status.json.tmp").exists());

        let plain = run(&shell, &tap_command(&status, None, "Ali's"), r#"{"second":true}"#);
        assert_eq!(plain, "Ali's");
        assert_eq!(fs::read_to_string(&status_file).unwrap(), r#"{"second":true}"#);
    }

    #[test]
    fn still_runs_the_wrapped_command_when_the_status_file_cannot_be_written() {
        let Some(shell) = shell() else {
            eprintln!("skipped: no Git Bash");
            return;
        };
        let missing = std::env::temp_dir().join("mc-does-not-exist").join("nested").join("x.status.json");
        assert_eq!(run(&shell, &tap_command(&missing.to_string_lossy(), Some("cat"), "Work"), "hello"), "hello");
    }
}
