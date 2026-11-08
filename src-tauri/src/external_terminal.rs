//! Opening an account's Claude in a terminal of its own (`openExternalSession` of
//! `src/main/index.ts` and `openExternalTerminal` of `src/main/externalTerminal.ts`). The session
//! runs outside the app: it has no tab, no hooks and no restart. A launcher script with the
//! account's config dir is written to `run/` and opened in Windows Terminal (or a console window),
//! Terminal.app or the system's terminal emulator. The scripts themselves are in [`crate::launchers`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use mc_core::error::{AppError, AppResult};
use mc_core::types::{AuthKind, CreateSessionInput, HookNeed, ResumeMode};

use crate::app::launch::{prepare_settings, CLAUDE_NOT_FOUND};
use crate::app::now_ms;
use crate::launchers::LauncherOptions;
use crate::sessions::title_of;
use crate::state::AppState;

/// What a terminal of its own is opened with.
#[derive(Debug, Clone)]
pub struct ExternalLaunch {
    /// The launcher script's account, claude, arguments and folder.
    pub options: LauncherOptions,
    pub cwd: String,
    /// The tab's title and color in Windows Terminal.
    pub title: String,
    pub color: String,
    /// Where the script is written (`run/`).
    pub script_dir: PathBuf,
    /// The whole environment of the terminal process.
    pub env: BTreeMap<String, String>,
}

/// `;` separates commands on the wt command line.
fn wt_escape(value: &str) -> String {
    value.replace(';', "\\;")
}

/// `wtArgs`: a new tab in the most recent Windows Terminal window, titled and colored for the
/// account, in the folder, running the script in cmd.exe.
pub fn wt_args(title: &str, color: &str, cwd: &str, script: &str) -> Vec<String> {
    vec![
        "-w".into(),
        "0".into(),
        "new-tab".into(),
        "--title".into(),
        wt_escape(title),
        "--tabColor".into(),
        color.into(),
        "-d".into(),
        wt_escape(cwd),
        "cmd.exe".into(),
        "/k".into(),
        wt_escape(script),
    ]
}

/// A process that is not tied to the app: no console of the app's, no input or output, and
/// nobody waiting for it (Node's `detached` with `stdio: 'ignore'`).
fn detached(program: &str, env: &BTreeMap<String, String>) -> Command {
    let mut command = Command::new(program);
    command.env_clear().envs(env).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
}

/// Starts it and lets it go; on Unix a thread collects its exit status.
fn launch(mut command: Command) -> io::Result<()> {
    let child = command.spawn()?;
    #[cfg(unix)]
    {
        let mut child = child;
        std::thread::Builder::new().name("external-terminal".into()).spawn(move || drop(child.wait()))?;
    }
    #[cfg(not(unix))]
    drop(child);
    Ok(())
}

fn script_base(script_dir: &Path) -> PathBuf {
    script_dir.join(format!("ext-{}", now_ms()))
}

/// `openExternalTerminal`: writes the launcher script and opens a terminal on it.
pub fn open_external_terminal(o: &ExternalLaunch) -> io::Result<()> {
    fs::create_dir_all(&o.script_dir)?;
    let base = script_base(&o.script_dir);
    #[cfg(windows)]
    {
        let mut script = base.into_os_string();
        script.push(".cmd");
        let script = PathBuf::from(script);
        fs::write(&script, crate::launchers::cmd_launcher(&o.options))?;
        let script = script.to_string_lossy().into_owned();
        let mut wt = detached("wt.exe", &o.env);
        wt.args(wt_args(&o.title, &o.color, &o.cwd, &script));
        if let Err(e) = launch(wt) {
            // no Windows Terminal: a plain console window
            log::info!("wt.exe: {e}; opening a console window instead");
            use std::os::windows::process::CommandExt;
            let mut console = detached("cmd.exe", &o.env);
            console.raw_arg("/c").raw_arg(format!("start \"\" cmd.exe /k \"{script}\""));
            launch(console)?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let mut script = base.into_os_string();
        script.push(".command");
        let script = PathBuf::from(script);
        fs::write(&script, crate::launchers::sh_launcher(&o.options))?;
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
        }
        let mut opener = if cfg!(target_os = "macos") {
            let mut open = detached("open", &o.env);
            open.args(["-a", "Terminal"]);
            open
        } else {
            let mut terminal = detached("x-terminal-emulator", &o.env);
            terminal.arg("-e");
            terminal
        };
        opener.arg(&script);
        launch(opener)
    }
}

/// `openExternalSession`: the account's Claude in a terminal of its own, in `input.cwd`,
/// continuing or picking a conversation as asked. Refused for a token account, whose script would
/// have to carry the token in plain text. The session gets its usage tap but no hooks: nothing
/// here would know when that terminal is closed.
pub fn open_external_session(state: &AppState, input: &CreateSessionInput) -> AppResult<()> {
    let profile = state.profiles.get(&input.profile_id).ok_or_else(|| AppError::refused("Profile not found"))?;
    let claude = state.claude_path().ok_or_else(|| AppError::refused(CLAUDE_NOT_FOUND))?;
    if profile.auth == Some(AuthKind::Token) {
        return Err(AppError::refused("Token accounts cannot be opened in an external terminal."));
    }
    let settings = prepare_settings(state, &profile, &format!("ext{}", now_ms()), Path::new(&input.cwd), HookNeed::Off)?;
    let mut args = Vec::new();
    if let Some(file) = settings {
        args.push("--settings".to_owned());
        args.push(file.to_string_lossy().into_owned());
    }
    match input.resume {
        Some(ResumeMode::Last) => args.push("-c".to_owned()),
        Some(ResumeMode::Pick) => args.push("--resume".to_owned()),
        None => {}
    }
    let launch = ExternalLaunch {
        options: LauncherOptions { config_dir: profile.config_dir.clone(), claude_path: claude, args, cwd: Some(input.cwd.clone()), pass_args: false },
        cwd: input.cwd.clone(),
        title: format!("{} · {}", profile.name, title_of(&input.cwd)),
        color: profile.color.clone(),
        script_dir: state.profiles.paths().run_dir(),
        env: state.profiles.env_for(&profile),
    };
    open_external_terminal(&launch).map_err(|e| AppError::io(format!("Could not open a terminal: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_wt_command_separator() {
        let args = wt_args("Work; rm", "#5b9bd5", "D:\\a;b", "C:\\run\\ext.cmd");
        assert_eq!(args, ["-w", "0", "new-tab", "--title", "Work\\; rm", "--tabColor", "#5b9bd5", "-d", "D:\\a\\;b", "cmd.exe", "/k", "C:\\run\\ext.cmd"]);
    }
}
