//! The text of a SKILL.md (`src/shared/skillText.ts`): a YAML frontmatter between two `---`
//! lines, then Markdown. Reading and changing such a text is [`crate::frontmatter`]; what is here
//! knows what a skill is.

use crate::frontmatter::{field_bool, field_text, frontmatter_block, name_of, parse_doc, set_field, Doc, FieldInput, YamlValue};
use crate::jsstr::len16;
use crate::types::{IssueLevel, IssueScope, SkillIssue, SKILL_LIMITS, SKILL_SWITCHES, UPLOAD_KEYS};

use super::new_name_problem;

/// JavaScript's white space for `trim()`: Unicode white space but U+0085, and the byte order mark.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `newSkillText`: the text of a skill that was just made.
pub fn new_skill_text(name: &str, description: &str) -> String {
    let title = name
        .split('-')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let description = match js_trim(description) {
        "" => "What this skill does, and when Claude should use it.",
        given => given,
    };
    let head = frontmatter_block(&[("name", YamlValue::from(name)), ("description", YamlValue::from(description))]);
    let title = if title.is_empty() { name } else { &title };
    format!("{head}\n# {title}\n\nWrite here what Claude should do when this skill is used.\n")
}

/// `renameInText`: the text of a skill whose folder gets another name. A name in the file follows
/// the folder when it was the folder's name (`always`: whenever the file has one, as for a copy
/// that must not answer to the name of its original).
pub fn rename_in_text(text: &str, old_dir_name: &str, new_dir_name: &str, always: bool) -> String {
    let doc = parse_doc(text);
    let Some(name) = name_of(&doc) else { return text.to_owned() };
    if doc.error.is_some() || (!always && name != old_dir_name) || name == new_dir_name {
        return text.to_owned();
    }
    set_field(text, "name", FieldInput::from(new_dir_name)).unwrap_or_else(|_| text.to_owned())
}

/// Who may start a skill, as its two switches say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation {
    /// Claude may pick the skill up by itself.
    pub model_invocable: bool,
    /// The skill shows in the / menu and runs as /name.
    pub user_invocable: bool,
}

/// `invocationOf`.
pub fn invocation_of(doc: &Doc<'_>) -> Invocation {
    Invocation {
        model_invocable: !field_bool(doc, SKILL_SWITCHES.hide_from_model, false),
        user_invocable: field_bool(doc, SKILL_SWITCHES.user_invocable, true),
    }
}

/// `inspectSkill`: what is wrong with a skill in the folder `dir_name`, or worth knowing. `claude`
/// issues matter to Claude Code; `upload` issues only when the skill is to be uploaded to
/// claude.ai, whose rules are stricter.
pub fn inspect_skill(doc: &Doc<'_>, dir_name: &str) -> Vec<SkillIssue> {
    let mut issues = Vec::new();
    let mut add = |level: IssueLevel, scope: IssueScope, message: String| issues.push(SkillIssue { level, scope, message, clash: false });

    if let Some(error) = &doc.error {
        let place = error.line.map(|line| format!(" (line {line})")).unwrap_or_default();
        let said = if error.message.ends_with(['.', '!', '?']) { error.message.clone() } else { format!("{}.", error.message) };
        add(IssueLevel::Error, IssueScope::Claude, format!("The frontmatter cannot be read{place}: {said} Claude Code then uses the skill without any of its fields."));
        return issues;
    }

    let description_field = field_text(doc, "description", false);
    let description = js_trim(&description_field);
    let listed = len16(description) + len16(js_trim(&field_text(doc, "when_to_use", false)));
    if description.is_empty() {
        add(IssueLevel::Warning, IssueScope::Claude, "No description. Claude goes by the first line of the text instead, which rarely says when to use the skill.".to_owned());
    } else if listed > SKILL_LIMITS.listing {
        add(
            IssueLevel::Warning,
            IssueScope::Claude,
            format!("Description and \"when to use\" have {listed} characters together; Claude sees the first {}.", SKILL_LIMITS.listing),
        );
    }
    if doc.lines > SKILL_LIMITS.lines {
        add(
            IssueLevel::Info,
            IssueScope::Claude,
            format!("SKILL.md has {} lines. Anthropic advises to stay under {} and to move detail into files next to it.", doc.lines, SKILL_LIMITS.lines),
        );
    }

    match name_of(doc) {
        None => add(IssueLevel::Warning, IssueScope::Upload, "No name in the file. Claude Code uses the folder name; claude.ai wants the name written down.".to_owned()),
        Some(name) => {
            if let Some(problem) = new_name_problem(&name) {
                add(IssueLevel::Warning, IssueScope::Upload, format!("The name \"{name}\" does not pass on claude.ai: {problem}"));
            } else if name != dir_name {
                add(
                    IssueLevel::Warning,
                    IssueScope::Upload,
                    format!("The name \"{name}\" differs from the folder \"{dir_name}\". Claude Code answers to both; claude.ai wants them equal."),
                );
            }
        }
    }
    let length = len16(description);
    if length > SKILL_LIMITS.description {
        add(IssueLevel::Warning, IssueScope::Upload, format!("The description has {length} characters; claude.ai takes at most {}.", SKILL_LIMITS.description));
    }
    if description.contains(['<', '>']) {
        add(IssueLevel::Warning, IssueScope::Upload, "The description contains < or >, which claude.ai refuses.".to_owned());
    }
    if len16(&field_text(doc, "compatibility", false)) > SKILL_LIMITS.compatibility {
        add(IssueLevel::Warning, IssueScope::Upload, format!("\"compatibility\" is longer than {} characters.", SKILL_LIMITS.compatibility));
    }
    let extra: Vec<&str> = doc.keys.iter().map(String::as_str).filter(|key| !UPLOAD_KEYS.contains(key)).collect();
    if !extra.is_empty() {
        let which = if extra.len() == 1 { "this field" } else { "these fields" };
        add(IssueLevel::Warning, IssueScope::Upload, format!("claude.ai does not accept {which}: {}. Claude Code uses them.", extra.join(", ")));
    }
    issues
}
