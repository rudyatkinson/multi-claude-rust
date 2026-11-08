//! The pure parts of `src/main/workflows/launcher.ts`: how a step's files are named and written,
//! the message its session starts with, what the variables of its prompt stand for, and the
//! removal of old run folders. Opening the session itself (`startStep`) needs the sessions and
//! lives in the Tauri crate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use super::engine::RunData;
use super::schema::{same_name, PromptValues};
use crate::error::{AppError, AppResult};
use crate::turn::{js_trim, JS_SPACE_CLASS};

/// Folders of runs older than this are removed when the app starts.
pub const KEEP_RUN_FILES_MS: i64 = 14 * 24 * 60 * 60 * 1000;
/// How much of a step's name goes into its file names.
const STEM_SLUG_MAX: usize = 30;

/// `foldText` of `src/shared/conversations.ts`: lower case and without diacritics. A private
/// copy, as `crate::conversations` belongs to another package. The dotless i is a letter of its
/// own, not an i with a mark, so it is the one letter that taking the marks off does not reach.
fn fold_text(text: &str) -> String {
    let bare: String = text.nfd().filter(|c| !is_combining_mark(*c)).collect();
    bare.to_lowercase().replace('\u{131}', "i")
}

/// "2-implement": a step's files sort in the order of the steps and still say what they are.
/// The name is folded, every run of anything but `a-z0-9` is one `-`, there is no `-` at the
/// ends, and at most 30 characters of it are kept; `step` when nothing is left.
pub fn step_file_stem(index: usize, name: &str) -> String {
    let mut slug = String::new();
    let mut in_run = false;
    for c in fold_text(name).chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
            in_run = false;
        } else if !in_run {
            slug.push('-');
            in_run = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    // only ASCII is left, so bytes are characters
    let cut = &trimmed[..trimmed.len().min(STEM_SLUG_MAX)];
    format!("{}-{}", index + 1, if cut.is_empty() { "step" } else { cut })
}

/// `1-plan.prompt.md`: what the step's session is told.
pub fn prompt_file_name(index: usize, name: &str) -> String {
    format!("{}.prompt.md", step_file_stem(index, name))
}

/// `1-plan.md`: what the step answered, for the user to look at.
pub fn output_file_name(index: usize, name: &str) -> String {
    format!("{}.md", step_file_stem(index, name))
}

/// The folder that holds the prompts and outputs of one run.
pub fn run_folder(runs_dir: &Path, run_id: &str) -> PathBuf {
    runs_dir.join(run_id)
}

/// A run folder goes into the first message inside quotes (`@"..."`): one with a quote or a
/// line break in its path cannot be handed to Claude.
pub fn check_run_folder(dir: &Path) -> AppResult<()> {
    if dir.to_string_lossy().contains(['"', '\r', '\n']) {
        return Err(AppError::refused("The folder that holds the app data has a name that cannot be handed to Claude."));
    }
    Ok(())
}

/// What the variables in a step's prompt stand for at this point of the run: the output of the
/// step before, and of the earlier steps by name (the first earlier step of that name, in any
/// case).
pub fn prompt_values(run: &RunData, index: usize) -> PromptValues<'_> {
    PromptValues {
        task: &run.task,
        folder: &run.cwd,
        previous: index.checked_sub(1).and_then(|before| run.steps.get(before)).and_then(|s| s.output.as_deref()),
        step: Box::new(move |name| {
            run.steps.iter().take(index).find(|s| same_name(&s.def.name, name)).and_then(|s| s.output.as_deref())
        }),
    }
}

static NOT_PLAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N} _.,()+\-]").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!("{JS_SPACE_CLASS}+")).unwrap());

/// A name as it can stand inside quotes on a command line: letters, digits and ` _.,()+-`, the
/// rest a space, runs of spaces one, none at the ends.
fn plain(text: &str) -> String {
    let replaced = NOT_PLAIN.replace_all(text, " ");
    js_trim(&SPACES.replace_all(&replaced, " ")).to_string()
}

/// The message a step's session starts with. The prompt itself travels in a file: a command line
/// has a length limit and quoting rules, a file has neither. Claude Code attaches a file that is
/// named with `@`, up to a size; the last sentence covers what is larger. `prompt_file` is the
/// path as it is shown to Claude.
pub fn launch_message(workflow_name: &str, step_name: &str, prompt_file: &str) -> String {
    format!(
        "Workflow \"{}\", step \"{}\": follow the instructions in @\"{prompt_file}\". If that file is shown to you only in part, read the rest of it before you start.",
        plain(workflow_name),
        plain(step_name)
    )
}

fn write_file(file: &Path, text: &str) -> AppResult<()> {
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| AppError::io(format!("Could not create {}: {e}", parent.display())))?;
    }
    fs::write(file, text).map_err(|e| AppError::io(format!("Could not write {}: {e}", file.display())))
}

/// Writes the rendered prompt of the step at `index` into the run folder `dir` (made when
/// missing) and gives the file's path. A folder that cannot be handed to Claude is refused first.
pub fn write_step_prompt(dir: &Path, index: usize, name: &str, text: &str) -> AppResult<PathBuf> {
    check_run_folder(dir)?;
    let file = dir.join(prompt_file_name(index, name));
    write_file(&file, text)?;
    Ok(file)
}

/// Writes a step's output beside its prompt, for the user to look at. Failing is harmless; the
/// caller may ignore the error.
pub fn write_step_output(dir: &Path, index: usize, name: &str, text: &str) -> AppResult<PathBuf> {
    let file = dir.join(output_file_name(index, name));
    write_file(&file, text)?;
    Ok(file)
}

/// Removes an entry of the runs folder: a folder with everything in it, or a file or a link
/// (never what a link points at).
fn remove_entry(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        // a link to a folder is a folder to Windows
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    }
}

/// Removes what is in `runs_dir` and was last changed more than `max_age_ms` (normally
/// [`KEEP_RUN_FILES_MS`]) before `now` (epoch ms). A folder that is not there, an entry that is
/// gone already or in use, is no reason to fail.
pub fn prune_run_folders(runs_dir: &Path, now: i64, max_age_ms: i64) {
    let Ok(entries) = fs::read_dir(runs_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) else { continue };
        let mtime_ms = match modified.duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs_f64() * 1000.0,
            Err(before) => -(before.duration().as_secs_f64() * 1000.0),
        };
        if now as f64 - mtime_ms > max_age_ms as f64 {
            let _ = remove_entry(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::types::{Extra, HookSignal, PermissionMode, Workflow, WorkflowStep};
    use crate::workflow::engine::{new_run, reduce, RunEvent};

    fn step(id: &str, name: &str, prompt: &str) -> WorkflowStep {
        WorkflowStep {
            id: id.into(),
            name: name.into(),
            profile_id: "main".into(),
            model: None,
            effort: None,
            permission_mode: None,
            continues: None,
            prompt: prompt.into(),
            gate: true,
            extra: Extra::new(),
        }
    }
    fn three() -> Workflow {
        let mut plan = step("plan", "Plan", "Plan: {{task}} in {{folder}}");
        plan.model = Some("fable".into());
        plan.permission_mode = Some(PermissionMode::Plan);
        let mut build = step("build", "Build it", "Build:\n{{previous.output}}");
        build.profile_id = "work".into();
        let mut review = step("review", "Review", "Review {{steps.Build it.output}} against {{steps.plan.output}}");
        review.continues = Some("plan".into());
        Workflow { id: "w1".into(), name: "Plan & \"ship\"".into(), steps: vec![plan, build, review], extra: Extra::new() }
    }
    fn play(mut run: RunData, events: &[RunEvent]) -> RunData {
        for event in events {
            run = reduce(&run, event, 0).run.into_owned();
        }
        run
    }
    fn answered(run: RunData, index: usize, tab: &str, text: &str) -> RunData {
        let hook = |signal| RunEvent::Hook { session_id: tab.into(), conversation_id: None, signal };
        play(
            run,
            &[
                RunEvent::Started { index, session_id: tab.into(), conversation_id: None },
                hook(HookSignal::Prompt),
                hook(HookSignal::Stop { text: text.into(), background: false }),
                RunEvent::Approve,
            ],
        )
    }

    /// Sets the time a folder was last changed.
    fn set_modified(dir: &Path, time: SystemTime) {
        #[cfg(windows)]
        let handle = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            fs::OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir).unwrap()
        };
        #[cfg(not(windows))]
        let handle = fs::File::open(dir).unwrap();
        handle.set_modified(time).unwrap();
    }

    #[test]
    fn names_a_steps_files_by_its_position_and_its_name() {
        assert_eq!(step_file_stem(0, "Plan"), "1-plan");
        assert_eq!(step_file_stem(1, "Build it_now-2"), "2-build-it-now-2");
        assert_eq!(step_file_stem(2, "İnceleme Işık"), "3-inceleme-isik");
        assert_eq!(step_file_stem(3, "計画"), "4-step");
        assert_eq!(prompt_file_name(0, "Plan"), "1-plan.prompt.md");
        assert_eq!(output_file_name(0, "Plan"), "1-plan.md");
    }

    #[test]
    fn writes_a_message_that_names_the_file_and_nothing_a_command_line_could_trip_over() {
        let message = launch_message(
            "Plan & \"ship\" `now`\n$(x)",
            "Step \"1\"",
            "C:\\Users\\My Name\\.multi-claude\\workflow-runs\\r1\\1-plan.prompt.md",
        );
        assert_eq!(
            message,
            "Workflow \"Plan ship now (x)\", step \"Step 1\": follow the instructions in @\"C:\\Users\\My Name\\.multi-claude\\workflow-runs\\r1\\1-plan.prompt.md\". If that file is shown to you only in part, read the rest of it before you start."
        );
    }

    #[test]
    fn knows_the_values_of_a_steps_variables() {
        let run = answered(play(new_run(&three(), "r", "/code", "T", 0), &[RunEvent::Begin]), 0, "t1", "OUT");
        let values = prompt_values(&run, 1);
        assert_eq!(
            (values.task, values.folder, values.previous, (values.step)("PLAN"), (values.step)("Build it"), (values.step)("Review")),
            ("T", "/code", Some("OUT"), Some("OUT"), None, None)
        );
        assert_eq!(prompt_values(&run, 0).previous, None);
    }

    #[test]
    fn removes_the_folders_of_runs_that_are_long_over() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = tmp.path().join("runs");
        let old = runs.join("old");
        let fresh = runs.join("fresh");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&fresh).unwrap();
        fs::write(old.join("1-plan.md"), "x").unwrap();
        let now = SystemTime::now();
        set_modified(&old, now - Duration::from_secs(15 * 86_400));
        let now_ms = now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        prune_run_folders(&runs, now_ms, KEEP_RUN_FILES_MS);
        let left: Vec<String> = fs::read_dir(&runs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["fresh"]);
        // a folder that is not there is no reason to fail
        prune_run_folders(&tmp.path().join("nothing"), now_ms, KEEP_RUN_FILES_MS);
    }

    #[test]
    fn writes_a_steps_prompt_and_its_output_beside_each_other() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = run_folder(&tmp.path().join("runs"), "run1");
        let prompt = write_step_prompt(&dir, 1, "Build it", "Build:\nTHE PLAN").unwrap();
        assert_eq!(prompt, dir.join("2-build-it.prompt.md"));
        assert_eq!(fs::read_to_string(&prompt).unwrap(), "Build:\nTHE PLAN");
        let output = write_step_output(&dir, 1, "Build it", "BUILT").unwrap();
        assert_eq!(fs::read_to_string(output).unwrap(), "BUILT");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn refuses_a_run_folder_that_cannot_be_handed_to_claude() {
        for bad in ["runs\\a\"b", "runs/a\nb", "runs/a\rb"] {
            let refused = write_step_prompt(Path::new(bad), 0, "Plan", "x").unwrap_err();
            assert_eq!(refused.to_string(), "The folder that holds the app data has a name that cannot be handed to Claude.");
        }
        assert!(check_run_folder(Path::new("C:\\Users\\My Name\\.multi-claude\\workflow-runs\\r1")).is_ok());
    }
}
