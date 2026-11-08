//! `bin/claude-<slug>` launcher scripts (`src/main/externalTerminal.ts`, launcher part).
//!
//! A launcher starts Claude Code with one account from any terminal. The account variables are
//! cleared inside the script because Windows Terminal starts new tabs with its own environment,
//! not the one of the process that asked for the tab. The external terminal opener uses the same
//! scripts with arguments and a folder ([`LauncherOptions`]).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::LazyLock;

use mc_core::env::AUTH_SCRUB;
use mc_core::types::{AuthKind, Profile};
use regex::Regex;

static CMD_SHIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(cmd|bat)$").expect("valid regex"));

/// True for a `.cmd` or `.bat` file, which cmd.exe has to run.
pub fn is_cmd_shim(path: &str) -> bool {
    CMD_SHIM.is_match(path)
}

/// What a launcher script runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LauncherOptions {
    /// None for Main: CLAUDE_CONFIG_DIR is cleared instead of set.
    pub config_dir: Option<String>,
    pub claude_path: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Forward the launcher's own arguments to claude.
    pub pass_args: bool,
}

impl LauncherOptions {
    /// A `claude-<slug>` launcher: no arguments of its own, the caller's passed on.
    pub fn launcher(claude_path: &str, config_dir: Option<&str>) -> Self {
        LauncherOptions {
            config_dir: config_dir.map(str::to_owned),
            claude_path: claude_path.to_owned(),
            args: Vec::new(),
            cwd: None,
            pass_args: true,
        }
    }
}

fn cmd_escape(value: &str) -> String {
    value.replace('%', "%%")
}

/// `shQuote`: one word for sh, in single quotes.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// `buildCmdLauncher`: a cmd script, CRLF line ends.
pub fn cmd_launcher(o: &LauncherOptions) -> String {
    let mut lines = vec!["@echo off".to_owned()];
    lines.extend(AUTH_SCRUB.iter().map(|name| format!("set \"{name}=\"")));
    if let Some(dir) = o.config_dir.as_deref().filter(|d| !d.is_empty()) {
        lines.push(format!("set \"CLAUDE_CONFIG_DIR={}\"", cmd_escape(dir)));
    }
    if let Some(cwd) = o.cwd.as_deref().filter(|d| !d.is_empty()) {
        lines.push(format!("cd /d \"{}\"", cmd_escape(cwd)));
    }
    let call = if is_cmd_shim(&o.claude_path) { "call " } else { "" };
    let mut run = vec![format!("{call}\"{}\"", cmd_escape(&o.claude_path))];
    run.extend(o.args.iter().map(|a| format!("\"{}\"", cmd_escape(a))));
    if o.pass_args {
        run.push("%*".to_owned());
    }
    lines.push(run.join(" "));
    // The file is UTF-8 and cmd reads it in the console code page, which only matters for non-ASCII paths.
    if !lines.iter().all(|l| l.is_ascii()) {
        lines.insert(1, "chcp 65001 >nul".to_owned());
    }
    lines.join("\r\n") + "\r\n"
}

/// `buildShLauncher`: a sh script.
pub fn sh_launcher(o: &LauncherOptions) -> String {
    let mut lines = vec!["#!/bin/sh".to_owned(), format!("unset {}", AUTH_SCRUB.join(" "))];
    if let Some(dir) = o.config_dir.as_deref().filter(|d| !d.is_empty()) {
        lines.push(format!("export CLAUDE_CONFIG_DIR={}", sh_quote(dir)));
    }
    if let Some(cwd) = o.cwd.as_deref().filter(|d| !d.is_empty()) {
        lines.push(format!("cd {} || exit 1", sh_quote(cwd)));
    }
    let mut run = vec!["exec".to_owned(), sh_quote(&o.claude_path)];
    run.extend(o.args.iter().map(|a| sh_quote(a)));
    if o.pass_args {
        run.push("\"$@\"".to_owned());
    }
    lines.push(run.join(" "));
    lines.join("\n") + "\n"
}

/// The launcher of this platform for `o`, and the file name extension it needs.
fn script_for(o: &LauncherOptions) -> (String, &'static str) {
    if cfg!(windows) {
        (cmd_launcher(o), ".cmd")
    } else {
        (sh_launcher(o), "")
    }
}

fn write_script(file: &Path, text: &str) -> io::Result<()> {
    fs::write(file, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// `writeLaunchers`: a `claude-<slug>` launcher per account in `bin_dir`, so each account can also
/// be started from any terminal; launchers of accounts that are gone are removed. A token account
/// gets none: its launcher would have to hold the token in plain text.
pub fn write_launchers(bin_dir: &Path, claude_path: &str, profiles: &[Profile]) -> io::Result<()> {
    let profiles: Vec<&Profile> = profiles.iter().filter(|p| p.auth != Some(AuthKind::Token)).collect();
    fs::create_dir_all(bin_dir)?;
    let ext = if cfg!(windows) { ".cmd" } else { "" };
    let wanted: HashSet<String> = profiles.iter().map(|p| format!("claude-{}{ext}", p.slug)).collect();
    for entry in fs::read_dir(bin_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("claude-") && !wanted.contains(&name) {
            if let Err(e) = fs::remove_file(entry.path()) {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e);
                }
            }
        }
    }
    for profile in profiles {
        let (text, ext) = script_for(&LauncherOptions::launcher(claude_path, profile.config_dir.as_deref()));
        write_script(&bin_dir.join(format!("claude-{}{ext}", profile.slug)), &text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts(config_dir: Option<&str>, claude: &str, args: &[&str], cwd: Option<&str>, pass_args: bool) -> LauncherOptions {
        LauncherOptions {
            config_dir: config_dir.map(str::to_owned),
            claude_path: claude.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: cwd.map(str::to_owned),
            pass_args,
        }
    }

    #[test]
    fn clears_account_variables_inside_the_cmd_script_and_sets_the_profile_dir() {
        let script = cmd_launcher(&opts(
            Some(r"C:\Users\me\.multi-claude\profiles\work"),
            r"C:\Users\me\.local\bin\claude.exe",
            &["--settings", r"C:\run\100% mine.json"],
            Some(r"D:\code\app"),
            false,
        ));
        let lines: Vec<&str> = script.trim_end().split("\r\n").collect();
        assert!(lines.contains(&r#"set "CLAUDE_CODE_OAUTH_TOKEN=""#));
        assert!(lines.contains(&r#"set "ANTHROPIC_API_KEY=""#));
        let cleared = lines.iter().position(|l| *l == r#"set "CLAUDE_CONFIG_DIR=""#).unwrap();
        let set = lines.iter().position(|l| *l == r#"set "CLAUDE_CONFIG_DIR=C:\Users\me\.multi-claude\profiles\work""#).unwrap();
        assert!(cleared < set);
        assert!(lines.contains(&r#"cd /d "D:\code\app""#));
        assert_eq!(*lines.last().unwrap(), r#""C:\Users\me\.local\bin\claude.exe" "--settings" "C:\run\100%% mine.json""#);
    }

    #[test]
    fn leaves_claude_config_dir_cleared_for_main_and_forwards_arguments_on_request() {
        let script = cmd_launcher(&opts(None, r"C:\npm\claude.cmd", &[], None, true));
        assert!(script.contains("set \"CLAUDE_CONFIG_DIR=\"\r\n"));
        assert!(!Regex::new(r#"set "CLAUDE_CONFIG_DIR=[^"]"#).unwrap().is_match(&script));
        assert_eq!(script.trim_end().split("\r\n").last().unwrap(), r#"call "C:\npm\claude.cmd" %*"#);
        assert_eq!(cmd_launcher(&LauncherOptions::launcher(r"C:\npm\claude.cmd", None)), script);
    }

    #[test]
    fn switches_the_code_page_only_for_non_ascii_paths() {
        let ascii = cmd_launcher(&opts(None, r"C:\bin\claude.exe", &[], None, false));
        let turkish = cmd_launcher(&opts(None, r"C:\bin\claude.exe", &[], Some(r"C:\Users\Çağrı\iş"), false));
        assert!(!ascii.contains("chcp"));
        assert_eq!(turkish.split("\r\n").nth(1).unwrap(), "chcp 65001 >nul");
        assert!(ascii.starts_with("@echo off\r\n"));
    }

    #[test]
    fn quotes_the_shell_launcher() {
        let script = sh_launcher(&opts(Some("/home/o'neil/.mc/work"), "/usr/local/bin/claude", &["-c"], Some("/tmp/my dir"), true));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("unset CLAUDE_CODE_OAUTH_TOKEN ANTHROPIC_API_KEY"));
        assert!(script.contains(r"export CLAUDE_CONFIG_DIR='/home/o'\''neil/.mc/work'"));
        assert!(script.contains("cd '/tmp/my dir' || exit 1"));
        assert_eq!(script.trim_end().rsplit('\n').next().unwrap(), r#"exec '/usr/local/bin/claude' '-c' "$@""#);
    }

    fn profile(slug: &str, config_dir: Option<&str>, token: bool) -> Profile {
        let mut v = json!({
            "id": slug, "name": slug, "slug": slug, "color": "#5b9bd5", "isDefault": config_dir.is_none(),
            "configDir": config_dir, "share": { "dirs": [], "files": [], "localSettingsKeys": [] }
        });
        if token {
            v["auth"] = json!("token");
        }
        serde_json::from_value(v).unwrap()
    }

    fn stems(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| Path::new(&e.unwrap().file_name()).file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn token_accounts_get_no_launcher_script() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        write_launchers(&bin, r"C:\bin\claude.exe", &[profile("main", None, false), profile("ci", Some(r"C:\profiles\ci"), true)]).unwrap();
        assert_eq!(stems(&bin), vec!["claude-main"]);
    }

    #[test]
    fn removes_launchers_of_accounts_that_are_gone_and_leaves_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().to_path_buf();
        let ext = if cfg!(windows) { ".cmd" } else { "" };
        fs::write(bin.join(format!("claude-old{ext}")), "x").unwrap();
        fs::write(bin.join("readme.txt"), "keep").unwrap();
        write_launchers(&bin, "/usr/bin/claude", &[profile("main", None, false), profile("work", Some("/p/work"), false)]).unwrap();
        assert_eq!(stems(&bin), vec!["claude-main", "claude-work", "readme"]);
        let work = fs::read_to_string(bin.join(format!("claude-work{ext}"))).unwrap();
        assert!(work.contains("work"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(bin.join("claude-work")).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }
}
