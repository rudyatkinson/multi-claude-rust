//! What a sleeping tab still shows after the app was closed: the output it fell asleep with and
//! what its conversation is about.
//!
//! The output is kept in `<root>/replays/<conversation id>.txt`, written once when the tab falls
//! asleep (it does not change while it sleeps) and read when the tab comes back asleep in the
//! next run. Files of tabs that did not come back asleep are removed then. The topic is not
//! kept: it is read from the end of the conversation file, as the history dialog reads it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mc_core::conversations::{read_tail, TITLE_CHARS};
use mc_core::jsstr::cut16;

use crate::sessions::Preview;

/// The folder of the kept outputs.
pub fn replay_dir(root: &Path) -> PathBuf {
    root.join("replays")
}

/// The file of one conversation's output; None for an id that is not one of Claude Code's (it
/// becomes a file name).
fn replay_file(root: &Path, claude_session_id: &str) -> Option<PathBuf> {
    uuid::Uuid::parse_str(claude_session_id).ok()?;
    Some(replay_dir(root).join(format!("{claude_session_id}.txt")))
}

/// Keeps the output of a tab that fell asleep. An empty output removes what was kept.
pub fn save_replay(root: &Path, claude_session_id: &str, replay: &str) {
    let Some(file) = replay_file(root, claude_session_id) else { return };
    if replay.is_empty() {
        let _ = std::fs::remove_file(&file);
        return;
    }
    if let Err(e) = write_atomic(&file, replay) {
        // the tab then comes back with an empty terminal
        log::warn!("keeping the output of a sleeping tab in {}: {e}", file.display());
    }
}

fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    if std::fs::rename(&tmp, file).is_err() {
        // Windows can refuse the rename while another process has the target open; retry once.
        if let Err(e) = std::fs::rename(&tmp, file) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    Ok(())
}

/// The output kept for a conversation, if any.
pub fn load_replay(root: &Path, claude_session_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(replay_file(root, claude_session_id)?).ok()?;
    (!text.is_empty()).then_some(text)
}

/// Removes every kept output but those of the conversations in `keep`.
pub fn prune_replays(root: &Path, keep: &HashSet<String>) {
    let Ok(files) = std::fs::read_dir(replay_dir(root)) else { return };
    for file in files.flatten() {
        let path = file.path();
        let kept = path.extension().is_some_and(|e| e == "txt")
            && path.file_stem().and_then(|s| s.to_str()).is_some_and(|id| keep.contains(id));
        if !kept {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// What a sleeping tab says of its conversation: Claude Code's title of it, else the start of the
/// last message. Nothing for a file that cannot be read or says neither.
pub fn preview_of(transcript: &Path) -> Preview {
    let Ok(tail) = read_tail(transcript) else { return Preview::default() };
    let topic = tail.title.or_else(|| tail.last_prompt.as_deref().map(|p| cut16(p, TITLE_CHARS).to_string())).filter(|t| !t.is_empty());
    Preview { topic, last_prompt: tail.last_prompt.filter(|p| !p.is_empty()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "11111111-2222-4333-8444-555555555555";
    const OTHER: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

    fn root() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("mc-sleep-view-").tempdir().unwrap()
    }

    #[test]
    fn an_output_is_kept_and_read_back() {
        let dir = root();
        assert_eq!(load_replay(dir.path(), ID), None);
        save_replay(dir.path(), ID, "\x1b[1mhello\x1b[0m 😀\r\n");
        assert_eq!(load_replay(dir.path(), ID).as_deref(), Some("\x1b[1mhello\x1b[0m 😀\r\n"));
        // the newest one takes its place, and nothing temporary is left behind
        save_replay(dir.path(), ID, "again");
        assert_eq!(load_replay(dir.path(), ID).as_deref(), Some("again"));
        assert_eq!(std::fs::read_dir(replay_dir(dir.path())).unwrap().count(), 1);
        // an empty output is not kept
        save_replay(dir.path(), ID, "");
        assert_eq!(load_replay(dir.path(), ID), None);
        assert_eq!(std::fs::read_dir(replay_dir(dir.path())).unwrap().count(), 0);
    }

    #[test]
    fn an_id_that_is_not_a_conversation_names_no_file() {
        let dir = root();
        save_replay(dir.path(), "../tabs", "x");
        save_replay(dir.path(), "", "x");
        assert!(!replay_dir(dir.path()).exists());
        assert!(!dir.path().join("tabs.txt").exists());
        assert_eq!(load_replay(dir.path(), "../tabs"), None);
    }

    #[test]
    fn outputs_of_tabs_that_are_gone_are_removed() {
        let dir = root();
        save_replay(dir.path(), ID, "kept");
        save_replay(dir.path(), OTHER, "gone");
        std::fs::write(replay_dir(dir.path()).join(format!("{ID}.txt.tmp-1")), "left over").unwrap();
        prune_replays(dir.path(), &HashSet::from([ID.to_owned()]));
        assert_eq!(load_replay(dir.path(), ID).as_deref(), Some("kept"));
        assert_eq!(load_replay(dir.path(), OTHER), None);
        assert_eq!(std::fs::read_dir(replay_dir(dir.path())).unwrap().count(), 1);
        // no folder: nothing to do
        prune_replays(&dir.path().join("missing"), &HashSet::new());
    }

    #[test]
    fn the_topic_is_the_title_else_the_start_of_the_last_message() {
        let dir = root();
        let file = dir.path().join("c.jsonl");
        let long = "word ".repeat(40);
        std::fs::write(&file, format!("{}\n", serde_json::json!({ "type": "last-prompt", "lastPrompt": long }))).unwrap();
        let preview = preview_of(&file);
        let topic = preview.topic.unwrap();
        assert!(long.starts_with(&topic) && topic.encode_utf16().count() <= TITLE_CHARS);
        assert!(preview.last_prompt.is_some());

        let lines = [
            serde_json::json!({ "type": "last-prompt", "lastPrompt": "now run the tests" }),
            serde_json::json!({ "type": "ai-title", "aiTitle": "Fix the login flow" }),
        ];
        std::fs::write(&file, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
        assert_eq!(
            preview_of(&file),
            Preview { topic: Some("Fix the login flow".into()), last_prompt: Some("now run the tests".into()) }
        );
        // nothing to read
        assert_eq!(preview_of(&dir.path().join("missing.jsonl")), Preview::default());
    }
}
