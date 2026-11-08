//! Skill rules: names, the fields of the form, clashes between skills, the text of a SKILL.md
//! (`src/shared/skills.ts`, `src/shared/skillText.ts`).
//!
//! Claude Code skills as the skill manager sees them: a folder with a SKILL.md in an account's
//! `skills` folder ("personal") or in a project's `.claude/skills` ("project"). Skills that come
//! from plugins or are synced from claude.ai are not its business.
//!
//! The limits, the switch keys and the upload keys are in [`crate::types`] (`SKILL_LIMITS`,
//! `SKILL_SWITCHES`, `UPLOAD_KEYS`); reading and changing the frontmatter is
//! [`crate::frontmatter`]. What is here knows what a skill is.

mod collate;
mod text;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};

use crate::conversations::fold_text;
use crate::jsstr::{cut16, longer16};
use crate::types::{FormChoice, FormField, FormFieldKind, IssueLevel, IssueScope, SkillIssue, SkillRef, SkillRoot, SkillRootKind, SKILL_LIMITS, SKILL_SWITCHES};

pub use collate::locale_compare;
pub use text::{inspect_skill, invocation_of, new_skill_text, rename_in_text, Invocation};

/// `skillKey`: how a skill is named in maps, `<rootId>/<dirName>`.
pub fn skill_key(r: &SkillRef) -> String {
    key_of(&r.root_id, &r.dir_name)
}

fn key_of(root_id: &str, dir_name: &str) -> String {
    format!("{root_id}/{dir_name}")
}

// ---------- names ----------

/// Names Windows keeps for devices, whatever the extension: `^(con|prn|aux|nul|com[0-9]|lpt[0-9])$`
/// without regard to case.
pub fn is_device_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "con" | "prn" | "aux" | "nul" => true,
        _ => lower.len() == 4 && (lower.starts_with("com") || lower.starts_with("lpt")) && lower.as_bytes()[3].is_ascii_digit(),
    }
}

/// `isReservedSkillDir`: folder names Claude Code keeps for itself inside a skills folder.
pub fn is_reserved_skill_dir(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "synced" || lower == "anthropic-skills" || name.starts_with('.')
}

/// `newNameProblem`: why a name cannot be given to a new skill, or None when it can. The rule is
/// the one of the Agent Skills format, which also keeps the folder name harmless on every file
/// system.
pub fn new_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give the skill a name.".to_owned());
    }
    kebab_problem(name, SKILL_LIMITS.name).or_else(|| is_reserved_skill_dir(name).then(|| format!("\"{name}\" is a name Claude Code keeps for itself.")))
}

/// `kebabProblem`: why a text that is not empty cannot be a name in kebab-case that also names a
/// file, or None. `max` counts UTF-16 units, as `length` does.
pub fn kebab_problem(name: &str, max: usize) -> Option<String> {
    if longer16(name, max) {
        return Some(format!("A name has at most {max} characters."));
    }
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Some("Use lower-case letters, digits and hyphens only.".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Some("A name does not start or end with a hyphen, and has no two hyphens in a row.".to_owned());
    }
    if is_device_name(name) {
        return Some(format!("\"{name}\" is a name Windows keeps for a device."));
    }
    None
}

/// `slugifySkillName`: a name made from any text, so that what is typed as a title becomes a
/// name that passes.
pub fn slugify_skill_name(text: &str) -> String {
    let folded = fold_text(text);
    // /[^a-z0-9]+/g -> '-'
    let mut slug = String::with_capacity(folded.len());
    let mut gap = false;
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap {
                slug.push('-');
                gap = false;
            }
            slug.push(c);
        } else {
            gap = true;
        }
    }
    // a gap at the end would only be trimmed again
    let trimmed = slug.trim_start_matches('-');
    // the text is ASCII by now, so a cut at a byte is a cut at a character
    let cut = &trimmed[..trimmed.len().min(SKILL_LIMITS.name)];
    cut.trim_end_matches('-').to_owned()
}

/// `freeSkillName`: a name that is free among `taken` (letter case does not count): the wish
/// itself, or with a number after it.
pub fn free_skill_name<S: AsRef<str>>(wish: &str, taken: &[S]) -> String {
    let used: HashSet<String> = taken.iter().map(|n| n.as_ref().to_lowercase()).collect();
    if !used.contains(&wish.to_lowercase()) {
        return wish.to_owned();
    }
    let mut n = 2u32;
    loop {
        let suffix = format!("-{n}");
        let room = SKILL_LIMITS.name.saturating_sub(suffix.len());
        let name = format!("{}{suffix}", cut16(wish, room).trim_end_matches('-'));
        if !used.contains(&name.to_lowercase()) {
            return name;
        }
        n += 1;
    }
}

// ---------- the fields of a SKILL.md ----------

/// One field of the form as a static table row: what [`FormField`] holds, without allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FormFieldKind,
    pub hint: &'static str,
    pub placeholder: Option<&'static str>,
    /// For `choice`: `(value, label)`; the empty value stands for "not set". Empty otherwise.
    pub choices: &'static [(&'static str, &'static str)],
}

impl FieldSpec {
    /// The field as the renderer's `FormField` has it.
    pub fn to_form_field(&self) -> FormField {
        FormField {
            key: self.key.to_owned(),
            label: self.label.to_owned(),
            kind: self.kind,
            hint: self.hint.to_owned(),
            placeholder: self.placeholder.map(str::to_owned),
            choices: (!self.choices.is_empty())
                .then(|| self.choices.iter().map(|(value, label)| FormChoice { value: (*value).to_owned(), label: (*label).to_owned() }).collect()),
            accepts: None,
        }
    }
}

/// `SKILL_FIELDS`: the fields the form shows, in order. Everything else in a file is left to the
/// Source tab. The effort choices are `EFFORT_LEVELS` (a test keeps them in step).
pub const SKILL_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "description",
        label: "Description",
        kind: FormFieldKind::Long,
        hint: "What the skill does and when to use it. Claude goes by this text when it decides whether the skill fits.",
        placeholder: Some("Reviews a pull request for … Use when …"),
        choices: &[],
    },
    FieldSpec {
        key: "when_to_use",
        label: "When to use",
        kind: FormFieldKind::Long,
        hint: "More about when the skill applies. Claude reads it together with the description.",
        placeholder: None,
        choices: &[],
    },
    FieldSpec {
        key: "argument-hint",
        label: "Argument hint",
        kind: FormFieldKind::Text,
        hint: "Shown after the name in the / menu.",
        placeholder: Some("[issue-number]"),
        choices: &[],
    },
    FieldSpec {
        key: "allowed-tools",
        label: "Allowed tools",
        kind: FormFieldKind::List,
        hint: "Tools Claude may use without asking while the skill is active.",
        placeholder: Some("Read, Grep, Bash(git *)"),
        choices: &[],
    },
    FieldSpec {
        key: "disallowed-tools",
        label: "Disallowed tools",
        kind: FormFieldKind::List,
        hint: "Tools that are taken away while the skill is active.",
        placeholder: Some("Write, Edit"),
        choices: &[],
    },
    FieldSpec {
        key: "model",
        label: "Model",
        kind: FormFieldKind::Text,
        hint: "The model while the skill is active: an alias, a full id, or inherit.",
        placeholder: Some("inherit"),
        choices: &[],
    },
    FieldSpec {
        key: "effort",
        label: "Effort",
        kind: FormFieldKind::Choice,
        hint: "The effort while the skill is active.",
        placeholder: None,
        choices: &[("", "As the session"), ("low", "low"), ("medium", "medium"), ("high", "high"), ("xhigh", "xhigh"), ("max", "max")],
    },
    FieldSpec {
        key: "context",
        label: "Runs",
        kind: FormFieldKind::Choice,
        hint: "In a subagent the skill works without the conversation so far and hands back its result.",
        placeholder: None,
        choices: &[("", "In the conversation"), ("fork", "In a subagent")],
    },
    FieldSpec {
        key: "agent",
        label: "Subagent type",
        kind: FormFieldKind::Text,
        hint: "Which kind of agent runs the skill when it runs in a subagent.",
        placeholder: Some("general-purpose"),
        choices: &[],
    },
    FieldSpec {
        key: "paths",
        label: "Paths",
        kind: FormFieldKind::List,
        hint: "Globs that limit where Claude picks the skill up by itself.",
        placeholder: Some("src/**/*.ts, docs/**"),
        choices: &[],
    },
];

/// `SKILL_FIELDS` as the renderer's `FormField`s.
pub fn skill_fields() -> Vec<FormField> {
    SKILL_FIELDS.iter().map(FieldSpec::to_form_field).collect()
}

/// `HANDLED_KEYS`: keys the form or the manager handles itself, so they are not listed as "also in
/// the file": `name`, the form's fields and the two switches.
pub fn handled_keys() -> Vec<&'static str> {
    let mut keys = vec!["name"];
    keys.extend(SKILL_FIELDS.iter().map(|f| f.key));
    keys.extend([SKILL_SWITCHES.hide_from_model, SKILL_SWITCHES.user_invocable]);
    keys
}

// ---------- names that get in each other's way ----------

/// A skill as [`name_clashes`] needs it: where it is and the names it answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named<'a> {
    pub root_id: &'a str,
    pub dir_name: &'a str,
    /// The name in the file, or the folder name when the file gives none.
    pub name: &'a str,
}

impl Named<'_> {
    /// The folder name and the name in the file, in lower case, each once.
    fn names(&self) -> Vec<String> {
        let dir = self.dir_name.to_lowercase();
        let name = self.name.to_lowercase();
        if dir == name {
            vec![dir]
        } else {
            vec![dir, name]
        }
    }
}

/// `nameClashes`: warnings about skills that answer to the same name, by [`skill_key`]. Claude
/// Code runs a skill by its folder name and by the name in its file, and lets a personal skill win
/// over a project one. `accounts_of` names the accounts of a personal root, for the message.
pub fn name_clashes(skills: &[Named<'_>], roots: &[SkillRoot], accounts_of: impl Fn(&SkillRoot) -> String) -> BTreeMap<String, Vec<SkillIssue>> {
    let mut result: BTreeMap<String, Vec<SkillIssue>> = BTreeMap::new();
    let mut add = |skill: &Named<'_>, message: String| {
        result.entry(key_of(skill.root_id, skill.dir_name)).or_default().push(SkillIssue {
            level: IssueLevel::Warning,
            scope: IssueScope::Claude,
            message,
            clash: true,
        });
    };
    let kind_of = |id: &str| roots.iter().find(|r| r.id == id).map(|r| r.kind);
    let names: Vec<Vec<String>> = skills.iter().map(Named::names).collect();
    let shares = |a: usize, b: usize| names[b].iter().any(|n| names[a].contains(n));

    for (i, skill) in skills.iter().enumerate() {
        let twin = skills.iter().enumerate().find(|(j, other)| *j != i && other.root_id == skill.root_id && shares(i, *j));
        if let Some((_, twin)) = twin {
            add(skill, format!("\"{}\" in the same folder answers to the same name. Only one of the two is used.", twin.dir_name));
        }

        if kind_of(skill.root_id) != Some(SkillRootKind::Project) {
            continue;
        }
        let personal = skills.iter().enumerate().find(|(j, other)| kind_of(other.root_id) == Some(SkillRootKind::Personal) && shares(i, *j));
        if let Some((_, personal)) = personal {
            let accounts = roots.iter().find(|r| r.id == personal.root_id).map(&accounts_of).unwrap_or_default();
            let whose = if accounts.is_empty() { String::new() } else { format!(" ({accounts})") };
            add(skill, format!("The personal skill \"{}\"{whose} has the same name and is used instead of this one.", personal.dir_name));
        }
    }
    result
}
