//! File-system work for skills (`src/main/skills/files.ts`; decoding, encoding, the retry and the
//! atomic write are in `mc_core::text_file`). Every function takes the folders it works in as
//! arguments.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use mc_core::error::{AppError, AppResult};
use mc_core::skills::locale_compare;
use mc_core::text_file::{busy_message, retry_busy, BUSY_TRIES, BUSY_WAIT};
use mc_core::types::SkillFile;

/// Folders tools fill by themselves; their files are not the skill's.
const SKIP_DIRS: [&str; 3] = ["node_modules", "__pycache__", ".git"];
const MAX_DEPTH: usize = 8;
/// How many files a skill's detail lists.
pub const WALK_LIMIT: usize = 300;

/// The entries of a folder with names that are text, each with what it is (a link is neither a
/// file nor a folder here: links are not followed).
pub(crate) fn entries_of(dir: &Path) -> io::Result<Vec<(String, fs::DirEntry, fs::FileType)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else { continue };
        let Ok(kind) = entry.file_type() else { continue };
        out.push((name, entry, kind));
    }
    Ok(out)
}

/// `walkSkill`: the files of a skill, without its SKILL.md, for showing; and whether there are
/// more than `limit`. Links are not followed, and the folders tools fill by themselves are left
/// out. Folders come first, then files, each in the order of `localeCompare`.
pub fn walk_skill(dir: &Path, limit: usize) -> (Vec<SkillFile>, bool) {
    fn visit(dir: &Path, sub: &str, depth: usize, limit: usize, files: &mut Vec<SkillFile>) -> bool {
        let at = if sub.is_empty() { dir.to_path_buf() } else { dir.join(sub) };
        let Ok(mut entries) = entries_of(&at) else { return false };
        entries.sort_by(|(a, _, ka), (b, _, kb)| kb.is_dir().cmp(&ka.is_dir()).then_with(|| locale_compare(a, b)));
        for (name, entry, kind) in entries {
            let rel = if sub.is_empty() { name.clone() } else { format!("{sub}/{name}") };
            if kind.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && depth < MAX_DEPTH && visit(dir, &rel, depth + 1, limit, files) {
                    return true;
                }
            } else if kind.is_file() && rel != "SKILL.md" {
                if files.len() >= limit {
                    return true;
                }
                // gone since it was listed: 0
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                files.push(SkillFile { path: rel, size });
            }
        }
        false
    }
    let mut files = Vec::new();
    let more = visit(dir, "", 0, limit, &mut files);
    (files, more)
}

/// `copyTree`: copies the plain files and folders of a tree. Links inside it are left behind.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else if kind.is_file() {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// `entryNamed`: the entry of a folder that has this name in any spelling of upper and lower case.
pub fn entry_named<'a>(names: &'a [String], name: &str) -> Option<&'a str> {
    let lower = name.to_lowercase();
    names.iter().find(|n| n.to_lowercase() == lower).map(String::as_str)
}

/// The names in a folder; none when it cannot be read (it is made with its first skill).
pub fn names_in(dir: &Path) -> Vec<String> {
    fs::read_dir(dir).map(|entries| entries.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect()).unwrap_or_default()
}

fn taken(name: &str) -> AppError {
    AppError::refused(format!("There is already a skill named \"{name}\" there."))
}

/// `placeFolder`: builds a new skill folder out of sight and moves it into place in one step:
/// Claude Code, which watches the skills folder, never sees a skill that is half there. The
/// staging folder lies inside `root_dir`, so the last step never crosses a drive, and its name
/// starts with a dot, which Claude Code skips. Nothing is left behind when `fill` fails.
pub fn place_folder(root_dir: &Path, name: &str, fill: impl FnOnce(&Path) -> AppResult<()>) -> AppResult<PathBuf> {
    if let Some(existing) = entry_named(&names_in(root_dir), name) {
        return Err(taken(existing));
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    let stage = root_dir.join(format!(".mc-new-{}", &token[..8]));
    let built = stage.join(name);
    let placed = (|| {
        fs::create_dir_all(&built)?;
        fill(&built)?;
        let target = root_dir.join(name);
        // made by someone else in the meantime
        if entry_named(&names_in(root_dir), name).is_some() {
            return Err(taken(name));
        }
        retry_busy(|| fs::rename(&built, &target), BUSY_TRIES, BUSY_WAIT).map_err(|err| busy_message(err, "Putting the skill into place"))?;
        Ok(target)
    })();
    let _ = fs::remove_dir_all(&stage);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(file: &Path, content: &str) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out = names_in(dir);
        out.sort();
        out
    }

    #[test]
    fn builds_out_of_sight_and_leaves_only_the_finished_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("new").join(".claude").join("skills");
        let mut seen_while_building = Vec::new();
        let dir = place_folder(&root, "fresh", |built| {
            fs::write(built.join("SKILL.md"), "x")?;
            seen_while_building = names(&root);
            Ok(())
        })
        .unwrap();
        assert_eq!(dir, root.join("fresh"));
        // while it was built, the root showed only a folder Claude Code skips
        assert_eq!(seen_while_building.len(), 1);
        assert!(seen_while_building[0].starts_with(".mc-new-") && seen_while_building[0].len() == 16, "{seen_while_building:?}");
        assert_eq!(names(&root), ["fresh"]);
    }

    #[test]
    fn leaves_nothing_behind_when_building_fails_and_refuses_a_name_that_is_taken() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        write(&root.join("Taken").join("SKILL.md"), "x");
        let err = place_folder(&root, "broken", |_| Err(AppError::io("disk full"))).unwrap_err();
        assert_eq!(err.to_string(), "disk full");
        assert_eq!(names(&root), ["Taken"]);
        let err = place_folder(&root, "taken", |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("already a skill named \"Taken\""), "{err}");
        assert_eq!(names(&root), ["Taken"]);
    }

    #[test]
    fn lists_the_files_next_to_skill_md_without_the_folders_tools_fill() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skills").join("big");
        write(&dir.join("SKILL.md"), "x");
        write(&dir.join("references").join("api.md"), "api");
        write(&dir.join("scripts").join("run.py"), "print(1)");
        write(&dir.join("node_modules").join("x").join("index.js"), "");
        write(&dir.join("scripts").join("__pycache__").join("run.pyc"), "");
        write(&dir.join("README.md"), "hello");
        let file = |path: &str, size: u64| SkillFile { path: path.into(), size };
        assert_eq!(walk_skill(&dir, WALK_LIMIT), (vec![file("references/api.md", 3), file("scripts/run.py", 8), file("README.md", 5)], false));
        assert!(walk_skill(&dir, 2).1);
        assert_eq!(walk_skill(&dir, 2).0.len(), 2);
        assert_eq!(walk_skill(&tmp.path().join("missing"), WALK_LIMIT), (vec![], false));
    }

    #[test]
    fn does_not_follow_links_and_stops_eight_folders_down() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("skill");
        write(&dir.join("SKILL.md"), "x");
        let elsewhere = tmp.path().join("elsewhere");
        write(&elsewhere.join("secret.txt"), "x");
        crate::library::test_support::link(&elsewhere, &dir.join("linked"));
        let deep: PathBuf = (0..9).fold(dir.clone(), |p, i| p.join(format!("d{i}")));
        write(&deep.join("too-deep.txt"), "x");
        write(&dir.join("d0").join("d1").join("d2").join("d3").join("d4").join("d5").join("d6").join("d7").join("ok.txt"), "x");
        let (files, more) = walk_skill(&dir, WALK_LIMIT);
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["d0/d1/d2/d3/d4/d5/d6/d7/ok.txt"]);
        assert!(!more);
    }

    #[test]
    fn copies_files_and_folders_but_not_links() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        write(&from.join("SKILL.md"), "skill");
        write(&from.join("refs").join("a.md"), "a");
        let elsewhere = tmp.path().join("elsewhere");
        write(&elsewhere.join("x.txt"), "x");
        crate::library::test_support::link(&elsewhere, &from.join("linked"));
        let to = tmp.path().join("to");
        copy_tree(&from, &to).unwrap();
        assert_eq!(names(&to), ["SKILL.md", "refs"]);
        assert_eq!(fs::read_to_string(to.join("refs").join("a.md")).unwrap(), "a");
    }

    #[test]
    fn finds_an_entry_in_any_spelling() {
        let names = vec!["Review".to_owned(), "deploy".to_owned()];
        assert_eq!(entry_named(&names, "review"), Some("Review"));
        assert_eq!(entry_named(&names, "other"), None);
    }
}
