//! The `claude` command line of a session (`src/main/sessions.ts` `claudeArgs`).
//!
//! Every value is checked here, because a value that starts with a dash would be read as a flag.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::error::{AppError, AppResult};
use crate::types::{LaunchOptions, ResumeMode, SessionRequest, SessionResume, EFFORT_LEVELS, PERMISSION_MODES};

/// `MODEL` of `src/shared/workflows.ts`: an alias or a full model id.
static MODEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:\[\]-]{0,79}$").expect("valid regex"));

/// `isModel`: a model name `--model` may be given.
pub fn is_model(value: &str) -> bool {
    MODEL.is_match(value)
}

/// `path.isAbsolute` of Node for this platform. On Windows a path rooted without a drive
/// (`\foo`, `/foo`) counts as absolute, as Node counts it and `Path::is_absolute` does not.
fn is_absolute_js(p: &str) -> bool {
    let b = p.as_bytes();
    if cfg!(windows) {
        let sep = |c: u8| c == b'/' || c == b'\\';
        b.first().is_some_and(|&c| sep(c)) || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]))
    } else {
        b.first() == Some(&b'/')
    }
}

/// The arguments for `claude` in a session tab, and the id of the conversation when it is known
/// up front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeArgs {
    pub args: Vec<String>,
    pub claude_session_id: Option<String>,
}

/// `claudeArgs`, with a fresh UUID for a new conversation.
pub fn claude_args(request: &SessionRequest, settings_file: Option<&Path>) -> AppResult<ClaudeArgs> {
    claude_args_with(request.resume.as_ref(), request.launch.as_ref(), request.prompt.as_deref(), settings_file, || {
        uuid::Uuid::new_v4().to_string()
    })
}

/// `claudeArgs(request, settingsFile, newConversationId)`: `--settings`, the launch options
/// (`--model`, `--effort`, `--permission-mode`, `--add-dir` with its folders), then the
/// conversation (`--resume <id>`, `-c`, `--resume` or `--session-id <new id>`), and the first
/// message last, behind `--`.
pub fn claude_args_with(
    resume: Option<&SessionResume>,
    launch: Option<&LaunchOptions>,
    prompt: Option<&str>,
    settings_file: Option<&Path>,
    new_conversation_id: impl FnOnce() -> String,
) -> AppResult<ClaudeArgs> {
    let mut args: Vec<String> = Vec::new();
    if let Some(file) = settings_file.filter(|f| !f.as_os_str().is_empty()) {
        args.push("--settings".into());
        args.push(file.to_string_lossy().into_owned());
    }

    if let Some(launch) = launch {
        if let Some(model) = &launch.model {
            if !is_model(model) {
                return Err(AppError::invalid(format!("Invalid model: {model}")));
            }
            args.push("--model".into());
            args.push(model.clone());
        }
        if let Some(effort) = &launch.effort {
            if !EFFORT_LEVELS.iter().any(|e| e.as_str() == effort) {
                return Err(AppError::invalid(format!("Invalid effort: {effort}")));
            }
            args.push("--effort".into());
            args.push(effort.clone());
        }
        if let Some(mode) = &launch.permission_mode {
            if !PERMISSION_MODES.iter().any(|m| m.as_str() == mode) {
                return Err(AppError::invalid(format!("Invalid permission mode: {mode}")));
            }
            args.push("--permission-mode".into());
            args.push(mode.clone());
        }
        if let Some(dirs) = launch.add_dirs.as_ref().filter(|d| !d.is_empty()) {
            if !dirs.iter().all(|dir| is_absolute_js(dir)) {
                return Err(AppError::invalid("Invalid folder"));
            }
            // takes any number of folders: the flag that follows ends its list
            args.push("--add-dir".into());
            args.extend(dirs.iter().cloned());
        }
    }

    let claude_session_id = match resume {
        Some(SessionResume::Conversation(conversation)) => {
            args.push("--resume".into());
            args.push(conversation.id.clone());
            Some(conversation.id.clone())
        }
        Some(SessionResume::Mode(ResumeMode::Last)) => {
            args.push("-c".into());
            None
        }
        Some(SessionResume::Mode(ResumeMode::Pick)) => {
            // without an id Claude Code shows its own list of the folder's conversations
            args.push("--resume".into());
            None
        }
        None => {
            // a new conversation gets its id from us, so the tab can be reopened later
            let id = new_conversation_id();
            args.push("--session-id".into());
            args.push(id.clone());
            Some(id)
        }
    };
    // "--" first: the text is never an option, whatever it starts with
    if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
        args.push("--".into());
        args.push(prompt.to_owned());
    }
    Ok(ClaudeArgs { args, claude_session_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResumeConversation;

    fn new_id() -> String {
        "new-id".into()
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn by_id(id: &str) -> SessionResume {
        SessionResume::Conversation(ResumeConversation { id: id.into() })
    }

    const LAST: SessionResume = SessionResume::Mode(ResumeMode::Last);
    const PICK: SessionResume = SessionResume::Mode(ResumeMode::Pick);

    fn settings() -> Option<&'static Path> {
        Some(Path::new("S.json"))
    }

    fn launch(f: impl FnOnce(&mut LaunchOptions)) -> LaunchOptions {
        let mut options = LaunchOptions::default();
        f(&mut options);
        options
    }

    fn message(result: AppResult<ClaudeArgs>) -> String {
        result.expect_err("should be refused").to_string()
    }

    // ---------- test/hooks.test.ts: claudeArgs ----------

    #[test]
    fn builds_what_a_plain_tab_always_got() {
        assert_eq!(
            claude_args_with(None, None, None, settings(), new_id).unwrap(),
            ClaudeArgs { args: strings(&["--settings", "S.json", "--session-id", "new-id"]), claude_session_id: Some("new-id".into()) }
        );
        assert_eq!(
            claude_args_with(Some(&by_id("abc")), None, None, settings(), new_id).unwrap(),
            ClaudeArgs { args: strings(&["--settings", "S.json", "--resume", "abc"]), claude_session_id: Some("abc".into()) }
        );
        assert_eq!(
            claude_args_with(Some(&LAST), None, None, settings(), new_id).unwrap(),
            ClaudeArgs { args: strings(&["--settings", "S.json", "-c"]), claude_session_id: None }
        );
        assert_eq!(
            claude_args_with(Some(&PICK), None, None, None, new_id).unwrap(),
            ClaudeArgs { args: strings(&["--resume"]), claude_session_id: None }
        );
        assert_eq!(claude_args_with(None, None, None, None, new_id).unwrap().args, strings(&["--session-id", "new-id"]));
    }

    #[test]
    fn adds_the_launch_options_before_the_conversation_and_the_message_last() {
        // `C:\runs\one` is absolute only to Windows, as it was to `path.isAbsolute` there
        let dirs: Vec<String> = if cfg!(windows) { strings(&["C:\\runs\\one", "/tmp/two"]) } else { strings(&["/runs/one", "/tmp/two"]) };
        let options = launch(|o| {
            o.model = Some("opus".into());
            o.effort = Some("high".into());
            o.permission_mode = Some("plan".into());
            o.add_dirs = Some(dirs.clone());
            o.watch = Some(true);
        });
        let built = claude_args_with(Some(&by_id("abc")), Some(&options), Some("--help me"), settings(), new_id).unwrap();
        let mut expected = strings(&["--settings", "S.json", "--model", "opus", "--effort", "high", "--permission-mode", "plan", "--add-dir"]);
        expected.extend(dirs);
        expected.extend(strings(&["--resume", "abc", "--", "--help me"]));
        assert_eq!(built.args, expected);
        assert_eq!(built.claude_session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn ends_the_list_of_folders_with_a_flag_even_when_nothing_else_follows() {
        let options = launch(|o| o.add_dirs = Some(strings(&["/a"])));
        assert_eq!(
            claude_args_with(Some(&PICK), Some(&options), Some("go"), None, new_id).unwrap().args,
            strings(&["--add-dir", "/a", "--resume", "--", "go"])
        );
        assert_eq!(claude_args_with(None, Some(&options), None, None, new_id).unwrap().args, strings(&["--add-dir", "/a", "--session-id", "new-id"]));
    }

    #[test]
    fn hands_a_message_on_as_one_argument() {
        // PICK_UP_MESSAGE of the transfer package, as it was in the Electron version
        let text = "Your last turn was cut off by a usage limit before it was finished. The conversation now continues on another account. \
                    Pick the task up where it stopped: check what is already done, then go on. Do not start over.";
        assert!(!text.contains(['"', '\'', '`', '\r', '\n', '%', '^', '&', '|', '<', '>']));
        assert_eq!(claude_args_with(Some(&by_id("abc")), None, Some(text), None, new_id).unwrap().args, strings(&["--resume", "abc", "--", text]));
    }

    #[test]
    fn refuses_values_that_are_not_what_they_should_be() {
        let model = launch(|o| o.model = Some("--dangerously-skip-permissions".into()));
        assert!(message(claude_args_with(None, Some(&model), None, None, new_id)).starts_with("Invalid model"));
        let effort = launch(|o| o.effort = Some("max --print".into()));
        assert!(message(claude_args_with(None, Some(&effort), None, None, new_id)).starts_with("Invalid effort"));
        let mode = launch(|o| o.permission_mode = Some("bypassPermissions".into()));
        assert!(message(claude_args_with(None, Some(&mode), None, None, new_id)).starts_with("Invalid permission mode"));
        let relative = launch(|o| o.add_dirs = Some(strings(&["relative/dir"])));
        assert_eq!(message(claude_args_with(None, Some(&relative), None, None, new_id)), "Invalid folder");
        let flag = launch(|o| o.add_dirs = Some(strings(&["--print"])));
        assert_eq!(message(claude_args_with(None, Some(&flag), None, None, new_id)), "Invalid folder");
    }

    #[test]
    fn models_follow_the_pattern() {
        assert!(is_model("opus"));
        assert!(is_model("claude-opus-4-1-20250805"));
        assert!(is_model("claude-sonnet-4[1m]"));
        assert!(is_model("us.anthropic.claude:0"));
        assert!(!is_model(""));
        assert!(!is_model("-opus"));
        assert!(!is_model("opus\n"));
        assert!(!is_model(&format!("a{}", "b".repeat(80))));
        assert!(is_model(&format!("a{}", "b".repeat(79))));
    }

    #[test]
    fn an_empty_message_is_no_message() {
        assert_eq!(claude_args_with(Some(&LAST), None, Some(""), None, new_id).unwrap().args, strings(&["-c"]));
    }

    #[test]
    fn takes_its_parts_from_a_request() {
        let request = SessionRequest {
            profile_id: "p".into(),
            cwd: "/".into(),
            kind: crate::types::SessionKind::Claude,
            resume: Some(LAST),
            name: None,
            launch: None,
            prompt: Some("hi".into()),
        };
        assert_eq!(claude_args(&request, None).unwrap().args, strings(&["-c", "--", "hi"]));
        // a new conversation gets a UUID
        let fresh = SessionRequest { resume: None, prompt: None, ..request };
        let id = claude_args(&fresh, None).unwrap().claude_session_id.unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
    }
}
