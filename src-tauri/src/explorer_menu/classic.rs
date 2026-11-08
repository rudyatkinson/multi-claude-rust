//! Registry verbs: what the classic context menu ("Show more options" on Windows 11) shows for
//! folders (`src/main/explorerMenu/classic.ts`). Everything goes through `reg.exe`, with the
//! arguments passed as a list and without a shell, so `%V` and quotes arrive untouched.

use std::time::Duration;

use mc_core::error::{AppError, AppResult};

/// The keys the verb is written under: folders, and the background of an open folder.
pub const PARENTS: [&str; 2] = [r"HKCU\Software\Classes\Directory\shell", r"HKCU\Software\Classes\Directory\Background\shell"];

/// How long a `reg.exe` call may take.
const REG_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicVerb {
    /// Registry key name of the verb.
    pub key: String,
    pub title: String,
    pub exe: String,
    /// Arguments before the folder switch.
    pub prefix_args: Vec<String>,
}

/// The shell substitutes `%`-sequences in verb commands, so a literal one is doubled.
fn shell_escape(value: &str) -> String {
    value.replace('%', "%%")
}

/// `classicCommand`: `%V` is the clicked folder, or the open folder for a click on its background.
pub fn classic_command(verb: &ClassicVerb) -> String {
    let parts: Vec<String> = std::iter::once(&verb.exe).chain(&verb.prefix_args).map(|part| format!("\"{}\"", shell_escape(part))).collect();
    format!("{} \"--open-folder=%V\"", parts.join(" "))
}

/// `classicRegCommands`: the arguments of the `reg.exe` calls that create the verb under both
/// folder types: its title, its icon and its command.
pub fn classic_reg_commands(verb: &ClassicVerb) -> Vec<Vec<String>> {
    let command = classic_command(verb);
    PARENTS
        .iter()
        .flat_map(|parent| {
            let key = format!("{parent}\\{}", verb.key);
            let args = |list: &[&str]| list.iter().map(|a| (*a).to_owned()).collect::<Vec<String>>();
            [
                args(&["add", &key, "/ve", "/d", &verb.title, "/f"]),
                args(&["add", &key, "/v", "Icon", "/d", &verb.exe, "/f"]),
                args(&["add", &format!("{key}\\command"), "/ve", "/d", &command, "/f"]),
            ]
        })
        .collect()
}

/// What a `reg.exe` call gave.
#[derive(Debug, Clone, Default)]
pub struct RegOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `reg.exe` without a window. A call that cannot start or takes too long is not `ok`.
pub async fn reg(args: &[String]) -> RegOutput {
    let mut command = tokio::process::Command::new("reg.exe");
    command.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match tokio::time::timeout(REG_TIMEOUT, command.output()).await {
        Ok(Ok(out)) => RegOutput {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Ok(Err(e)) => RegOutput { ok: false, stdout: String::new(), stderr: e.to_string() },
        Err(_) => RegOutput { ok: false, stdout: String::new(), stderr: "reg.exe did not answer in time".into() },
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Writes the verb under both folder types.
pub async fn install_classic(verb: &ClassicVerb) -> AppResult<()> {
    for args in classic_reg_commands(verb) {
        let result = reg(&args).await;
        if !result.ok {
            let message = result.stderr.trim();
            return Err(AppError::io(if message.is_empty() { "Could not write to the registry" } else { message }));
        }
    }
    Ok(())
}

/// Removes the verb; a missing key is fine.
pub async fn remove_classic(key: &str) {
    for parent in PARENTS {
        reg(&strings(&["delete", &format!("{parent}\\{key}"), "/f"])).await;
    }
}

/// Whether the verb's command is there under both folder types.
pub async fn is_classic_installed(key: &str) -> bool {
    let [folders, backgrounds] = PARENTS.map(|parent| strings(&["query", &format!("{parent}\\{key}\\command"), "/ve"]));
    let (a, b) = tokio::join!(reg(&folders), reg(&backgrounds));
    a.ok && b.ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verb() -> ClassicVerb {
        ClassicVerb {
            key: "MultiClaude".into(),
            title: super::super::MENU_TITLE.into(),
            exe: r"C:\Program Files\Multi Claude\Multi Claude.exe".into(),
            prefix_args: vec![],
        }
    }

    #[test]
    fn quotes_the_executable_and_passes_the_folder_as_one_argument() {
        assert_eq!(classic_command(&verb()), r#""C:\Program Files\Multi Claude\Multi Claude.exe" "--open-folder=%V""#);
    }

    #[test]
    fn passes_arguments_before_the_folder_and_escapes_percent_signs() {
        let verb = ClassicVerb { exe: r"C:\100%\electron.exe".into(), prefix_args: vec![r"C:\src\multi-claude".into()], ..verb() };
        assert_eq!(classic_command(&verb), r#""C:\100%%\electron.exe" "C:\src\multi-claude" "--open-folder=%V""#);
    }

    #[test]
    fn writes_title_icon_and_command_for_folders_and_folder_backgrounds() {
        let verb = verb();
        let commands = classic_reg_commands(&verb);
        assert_eq!(commands.len(), 6);
        let mut keys: Vec<String> = Vec::new();
        for command in &commands {
            let key = command[1].strip_suffix(r"\command").unwrap_or(&command[1]).to_owned();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        assert_eq!(keys, [r"HKCU\Software\Classes\Directory\shell\MultiClaude", r"HKCU\Software\Classes\Directory\Background\shell\MultiClaude"]);
        assert_eq!(commands[0], ["add", keys[0].as_str(), "/ve", "/d", "Open in Multi Claude", "/f"]);
        assert_eq!(commands[1], ["add", keys[0].as_str(), "/v", "Icon", "/d", verb.exe.as_str(), "/f"]);
        assert_eq!(commands[2], ["add", format!(r"{}\command", keys[0]).as_str(), "/ve", "/d", classic_command(&verb).as_str(), "/f"]);
    }
}
