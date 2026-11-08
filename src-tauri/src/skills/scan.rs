//! The skills of a folder and what the list shows of each (`src/main/skills/scan.ts`).

use std::fs;
use std::path::Path;

use mc_core::frontmatter::{field_text, first_body_line, name_of, parse_doc};
use mc_core::jsstr::cut16;
use mc_core::skills::{inspect_skill, invocation_of, is_reserved_skill_dir, locale_compare};
use mc_core::text_file::decode;
use mc_core::types::{IssueLevel, IssueScope, SkillIssue, SkillSummary, SKILL_LIMITS};

use crate::library::roots::{path_key, plain_path, RootSpec};
use crate::skills::files::entries_of;

/// What a listing shows of a description.
const DESCRIPTION_CHARS: usize = 400;

/// A skill folder of a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSkill {
    pub dir_name: String,
    /// The folder is a link (a junction, a symlink) to a folder elsewhere.
    pub linked: bool,
}

/// `findSkills`: the skills of one folder: its direct subfolders (or links to folders) that hold a
/// SKILL.md, in the order of `localeCompare`. Left out are the folders Claude Code keeps for
/// itself (`synced` for claude.ai's skills, `anthropic-skills`, anything that starts with a dot),
/// folders that load as a plugin, and a second link to a folder that is listed already.
pub fn find_skills(root_dir: &Path) -> Vec<FoundSkill> {
    let Ok(mut entries) = entries_of(root_dir) else { return Vec::new() };
    entries.sort_by(|(a, _, _), (b, _, _)| locale_compare(a, b));
    let mut found = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (name, _, kind) in entries {
        if is_reserved_skill_dir(&name) {
            continue;
        }
        let dir = root_dir.join(&name);
        let linked = kind.is_symlink();
        if !kind.is_dir() && !linked {
            continue;
        }
        if !fs::metadata(dir.join("SKILL.md")).is_ok_and(|m| m.is_file()) {
            continue;
        }
        // a link that leads nowhere has no SKILL.md either
        let Ok(real) = fs::canonicalize(&dir) else { continue };
        let real = path_key(&plain_path(real)).to_lowercase();
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        if dir.join(".claude-plugin").join("plugin.json").exists() {
            continue;
        }
        found.push(FoundSkill { dir_name: name, linked });
    }
    found
}

/// JavaScript's `\s`.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `text.replace(/\s+/g, ' ')`.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars() {
        if is_js_space(c) {
            gap = true;
        } else {
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push(c);
        }
    }
    if gap {
        out.push(' ');
    }
    out
}

/// `summarize`: what the list shows of one skill. A file that cannot be read still gets its line.
pub fn summarize(root: &RootSpec, skill: &FoundSkill) -> SkillSummary {
    let base = |issue: SkillIssue| SkillSummary {
        root_id: root.id.clone(),
        dir_name: skill.dir_name.clone(),
        name: skill.dir_name.clone(),
        description: String::new(),
        model_invocable: true,
        user_invocable: true,
        linked: skill.linked,
        issues: vec![issue],
    };
    let issue = |level, message| SkillIssue { level, scope: IssueScope::Claude, message, clash: false };
    let bytes = match fs::read(root.dir.join(&skill.dir_name).join("SKILL.md")) {
        Ok(bytes) => bytes,
        Err(err) => return base(issue(IssueLevel::Error, format!("SKILL.md cannot be read: {err}"))),
    };
    let text = match decode(&bytes, SKILL_LIMITS.file_bytes) {
        Ok(decoded) => decoded.text,
        Err(unreadable) => return base(issue(IssueLevel::Warning, unreadable.0)),
    };
    let doc = parse_doc(&text);
    let described = field_text(&doc, "description", false);
    let described = described.trim_matches(is_js_space);
    let description = if described.is_empty() { first_body_line(doc.body) } else { described.to_owned() };
    let invocation = invocation_of(&doc);
    SkillSummary {
        root_id: root.id.clone(),
        dir_name: skill.dir_name.clone(),
        name: name_of(&doc).unwrap_or_else(|| skill.dir_name.clone()),
        description: cut16(&one_line(&description), DESCRIPTION_CHARS).to_owned(),
        model_invocable: invocation.model_invocable,
        user_invocable: invocation.user_invocable,
        linked: skill.linked,
        issues: inspect_skill(&doc, &skill.dir_name).into_iter().filter(|i| i.scope == IssueScope::Claude).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::library::test_support::link;

    fn write(file: &Path, content: &str) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn add_skill(root: &Path, dir_name: &str) -> PathBuf {
        write(&root.join(dir_name).join("SKILL.md"), &format!("---\nname: {dir_name}\ndescription: Does {dir_name}.\n---\n\n# {dir_name}\n"));
        root.join(dir_name)
    }

    fn found(dir_name: &str, linked: bool) -> FoundSkill {
        FoundSkill { dir_name: dir_name.into(), linked }
    }

    #[test]
    fn lists_folders_with_a_skill_md_and_skips_what_claude_code_keeps_for_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        add_skill(&root, "review");
        add_skill(&root, "Deploy");
        add_skill(&root.join("synced").join("bucket"), "from-claude-ai");
        add_skill(&root, "Synced");
        add_skill(&root, ".trash");
        add_skill(&root, ".mc-new-12345678");
        add_skill(&root, "anthropic-skills");
        fs::create_dir_all(root.join("no-skill-file")).unwrap();
        write(&root.join("loose.md"), "x");
        let as_plugin = add_skill(&root, "packaged");
        write(&as_plugin.join(".claude-plugin").join("plugin.json"), "{}");
        assert_eq!(find_skills(&root), [found("Deploy", false), found("review", false)]);
        assert_eq!(find_skills(&tmp.path().join("missing")), []);
    }

    #[test]
    fn lists_a_linked_skill_once_and_marks_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        let elsewhere = add_skill(&tmp.path().join("repo"), "shared-skill");
        fs::create_dir_all(&root).unwrap();
        link(&elsewhere, &root.join("a-link"));
        link(&elsewhere, &root.join("b-link"));
        link(&tmp.path().join("nowhere"), &root.join("dead-link"));
        assert_eq!(find_skills(&root), [found("a-link", true)]);
    }

    #[test]
    fn makes_one_line_of_a_description() {
        assert_eq!(one_line("a \n\t b  c"), "a b c");
        assert_eq!(one_line(" a "), " a ");
    }
}
