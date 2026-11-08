//! Moving a conversation between accounts: transcript lookup, plan, prefix check, copy
//! (`src/main/transfer.ts`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::conversations::is_uuid;
use crate::slug::is_inside;

/// What Claude is told on the account a conversation was moved to after a usage limit cut its turn
/// off. It travels as an argument of the command line, so it holds no quotes and no line breaks.
pub const PICK_UP_MESSAGE: &str = "Your last turn was cut off by a usage limit before it was finished. The conversation now continues on another account. \
     Pick the task up where it stopped: check what is already done, then go on. Do not start over.";

/// One file or folder to copy (`Copy` in the TypeScript, which would shadow the trait here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCopy {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPlan {
    /// The conversation itself.
    pub transcript: FileCopy,
    /// What belongs to it and exists: its subagent and tool result files, its file history.
    pub extras: Vec<FileCopy>,
}

/// The transcript of a conversation in a config dir: at the path Claude Code reported if that is
/// `<id>.jsonl` inside `<config dir>/projects`, else wherever a file of that name lies under
/// `projects/`. `None` for an id that is not a UUID, which is never used in a path.
pub fn find_transcript(config_dir: &Path, id: &str, reported: Option<&str>) -> Option<PathBuf> {
    if !is_uuid(id) {
        return None;
    }
    let projects = config_dir.join("projects");
    let name = format!("{id}.jsonl");
    if let Some(reported) = reported.filter(|r| !r.is_empty()).map(Path::new) {
        if reported.file_name().is_some_and(|n| n == name.as_str()) && is_inside(reported, &projects) && reported.exists() {
            return Some(reported.to_path_buf());
        }
    }
    // no conversations in this config dir when it cannot be read
    let mut keys: Vec<_> = fs::read_dir(&projects).ok()?.filter_map(|entry| entry.ok().map(|e| e.file_name())).collect();
    keys.sort();
    keys.into_iter().map(|key| projects.join(key).join(&name)).find(|file| file.exists())
}

/// Which files carry a conversation from one account's config dir into another's. The folder key
/// under `projects/` is taken over as it is, so Claude Code finds the conversation from the same
/// working folder. `None` when the source has no transcript.
pub fn plan_transfer(src_config: &Path, dst_config: &Path, id: &str, reported: Option<&str>) -> Option<TransferPlan> {
    let from = find_transcript(src_config, id, reported)?;
    let folder = from.parent()?;
    let dst_project = dst_config.join("projects").join(folder.file_name()?);
    let mut extras = Vec::new();
    let sibling = folder.join(id);
    if sibling.exists() {
        extras.push(FileCopy { from: sibling, to: dst_project.join(id) });
    }
    let history = src_config.join("file-history").join(id);
    if history.exists() {
        extras.push(FileCopy { from: history, to: dst_config.join("file-history").join(id) });
    }
    Some(TransferPlan { transcript: FileCopy { from, to: dst_project.join(format!("{id}.jsonl")) }, extras })
}

/// Whether the conversation at the destination may be replaced. It may when it is an earlier
/// state of the same conversation (it was moved back and forth), i.e. a byte prefix of the source;
/// it may not when it went on there in a different way, because that continuation would be lost.
/// An error when the source cannot be read.
pub fn can_replace(plan: &TransferPlan) -> io::Result<bool> {
    let Ok(existing) = fs::read(&plan.transcript.to) else {
        return Ok(true);
    };
    let incoming = fs::read(&plan.transcript.from)?;
    Ok(incoming.starts_with(&existing))
}

/// Copies the transcript and its extras, folders recursively, over whatever is at the
/// destination. The source stays in place.
pub fn copy_conversation(plan: &TransferPlan) -> io::Result<()> {
    for step in std::iter::once(&plan.transcript).chain(&plan.extras) {
        if let Some(parent) = step.to.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_tree(&step.from, &step.to)?;
    }
    Ok(())
}

/// `fs.cp(from, to, { recursive: true, force: true })`, except that links are followed: what they
/// point to is copied.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if !fs::metadata(from)?.is_dir() {
        return fs::copy(from, to).map(drop);
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        copy_tree(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "2bf04019-6df2-46c6-93d8-91c02cde383c";
    const KEY: &str = "C--code-my-app";

    fn write(file: &Path, text: &str) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    /// `main` holds the conversation, `elif` nothing yet.
    struct Accounts {
        root: tempfile::TempDir,
        src: PathBuf,
        dst: PathBuf,
    }

    impl Accounts {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let src = root.path().join("main");
            let dst = root.path().join("elif");
            write(&src.join("projects").join(KEY).join(format!("{ID}.jsonl")), "line 1\nline 2\n");
            Accounts { root, src, dst }
        }

        fn transcript(config: &Path) -> PathBuf {
            config.join("projects").join(KEY).join(format!("{ID}.jsonl"))
        }
    }

    fn text(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    // ---------- moving a conversation between accounts (test/trayAndTransfer.test.ts) ----------

    #[test]
    fn finds_the_transcript_at_the_reported_path_or_by_its_id() {
        let a = Accounts::new();
        let file = Accounts::transcript(&a.src);
        assert_eq!(find_transcript(&a.src, ID, file.to_str()), Some(file.clone()));
        assert_eq!(find_transcript(&a.src, ID, None), Some(file));
        assert_eq!(find_transcript(&a.dst, ID, None), None);
        assert_eq!(find_transcript(&a.src, "00000000-0000-0000-0000-000000000000", None), None);
    }

    #[test]
    fn does_not_follow_a_reported_path_outside_the_account_nor_an_id_that_is_not_one() {
        let a = Accounts::new();
        let outside = a.root.path().join("elsewhere").join(format!("{ID}.jsonl"));
        write(&outside, "x");
        // falls back to the search inside the config dir
        assert_eq!(find_transcript(&a.src, ID, outside.to_str()), Some(Accounts::transcript(&a.src)));
        assert_eq!(find_transcript(&a.dst, ID, outside.to_str()), None);
        assert_eq!(find_transcript(&a.src, r"..\..\secret", None), None);
        // nor a file of another name inside it
        let other = a.src.join("projects").join(KEY).join("other.jsonl");
        write(&other, "x");
        assert_eq!(find_transcript(&a.src, ID, other.to_str()), Some(Accounts::transcript(&a.src)));
    }

    #[test]
    fn plans_the_transcript_and_what_belongs_to_it_under_the_same_folder_key() {
        let a = Accounts::new();
        write(&a.src.join("projects").join(KEY).join(ID).join("subagents").join("a.jsonl"), "sub");
        write(&a.src.join("file-history").join(ID).join("f@v1"), "old file");
        let plan = plan_transfer(&a.src, &a.dst, ID, None).unwrap();
        assert_eq!(plan.transcript, FileCopy { from: Accounts::transcript(&a.src), to: Accounts::transcript(&a.dst) });
        assert_eq!(
            plan.extras,
            [
                FileCopy { from: a.src.join("projects").join(KEY).join(ID), to: a.dst.join("projects").join(KEY).join(ID) },
                FileCopy { from: a.src.join("file-history").join(ID), to: a.dst.join("file-history").join(ID) },
            ]
        );
    }

    #[test]
    fn has_nothing_to_plan_for_a_conversation_that_was_never_written() {
        let a = Accounts::new();
        assert_eq!(plan_transfer(&a.dst, &a.src, ID, None), None);
        assert!(plan_transfer(&a.src, &a.dst, ID, None).unwrap().extras.is_empty());
    }

    #[test]
    fn copies_everything_leaving_the_source_in_place() {
        let a = Accounts::new();
        write(&a.src.join("projects").join(KEY).join(ID).join("tool-results").join("r.txt"), "result");
        let plan = plan_transfer(&a.src, &a.dst, ID, None).unwrap();
        assert!(can_replace(&plan).unwrap());
        copy_conversation(&plan).unwrap();
        assert_eq!(text(&Accounts::transcript(&a.dst)), "line 1\nline 2\n");
        assert_eq!(text(&a.dst.join("projects").join(KEY).join(ID).join("tool-results").join("r.txt")), "result");
        assert!(Accounts::transcript(&a.src).exists());
    }

    #[test]
    fn replaces_an_earlier_state_of_the_same_conversation_at_the_destination() {
        let a = Accounts::new();
        // moved to the other account before, came back, went on here
        write(&Accounts::transcript(&a.dst), "line 1\n");
        let plan = plan_transfer(&a.src, &a.dst, ID, None).unwrap();
        assert!(can_replace(&plan).unwrap());
        copy_conversation(&plan).unwrap();
        assert_eq!(text(&plan.transcript.to), "line 1\nline 2\n");
    }

    #[test]
    fn refuses_to_overwrite_a_conversation_that_went_on_differently_at_the_destination() {
        let a = Accounts::new();
        write(&Accounts::transcript(&a.dst), "line 1\nanother line 2\n");
        assert!(!can_replace(&plan_transfer(&a.src, &a.dst, ID, None).unwrap()).unwrap());
        // also when the destination is simply ahead
        write(&Accounts::transcript(&a.dst), "line 1\nline 2\nline 3\n");
        assert!(!can_replace(&plan_transfer(&a.src, &a.dst, ID, None).unwrap()).unwrap());
    }

    #[test]
    fn says_what_claude_is_told_on_one_line_without_quotes() {
        assert!(!PICK_UP_MESSAGE.contains(['"', '\'', '\n']));
        assert!(PICK_UP_MESSAGE.contains("another account. Pick the task up"));
    }
}
