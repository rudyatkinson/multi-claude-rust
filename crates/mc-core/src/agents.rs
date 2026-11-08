//! Subagent rules: limits, fields, inspection, clashes (`src/shared/agents.ts`).
//!
//! Claude Code subagents as the Library sees them: a Markdown file with a frontmatter in an
//! account's `agents` folder ("personal") or in a project's `.claude/agents` ("project"), at any
//! depth. What names an agent is the `name` in its file, not the name of the file. Agents that are
//! built in or come from plugins are not its business.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::frontmatter::{field_text, frontmatter_block, name_of, Doc, YamlValue};
use crate::jsstr::{len16, longer16};
use crate::types::{
    AgentIssue, AgentRef, AgentRoot, FormChoice, FormField, FormFieldKind, IssueLevel, SkillRootKind, AGENT_LIMITS, EFFORT_LEVELS,
};

/// `agentKey`: one text per agent, `<rootId>/<file>`.
pub fn agent_key(root_id: &str, file: &str) -> String {
    format!("{root_id}/{file}")
}

/// `agentKey` of a ref.
pub fn ref_key(agent: &AgentRef) -> String {
    agent_key(&agent.root_id, &agent.file)
}

/// The file's name without its folders and its ending: what an agent is usually named after.
pub fn agent_stem(file: &str) -> &str {
    let base = file.rsplit('/').next().unwrap_or(file);
    match base.len().checked_sub(3).and_then(|at| base.get(at..).map(|end| (at, end))) {
        Some((at, end)) if end.eq_ignore_ascii_case(".md") => &base[..at],
        _ => base,
    }
}

/// `/^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/i`: a name Windows keeps for a device.
fn is_device_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || (lower.len() == 4 && (lower.starts_with("com") || lower.starts_with("lpt")) && lower.as_bytes()[3].is_ascii_digit())
}

/// `kebabProblem` of `src/shared/skills.ts`: why a text that is not empty cannot be a name in
/// kebab-case that also names a file, or None.
fn kebab_problem(name: &str, max: usize) -> Option<String> {
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

/// Why a name cannot be given to an agent here, or None when it can.
pub fn agent_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give the agent a name.".to_owned());
    }
    kebab_problem(name, AGENT_LIMITS.new_name)
}

// ---------- the fields of an agent file ----------

/// The permission modes the form offers.
pub const AGENT_PERMISSION_MODES: [&str; 6] = ["default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan"];
/// The colours an agent can show in.
pub const AGENT_COLORS: [&str; 8] = ["red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan"];

fn choices<'a>(unset: &str, values: impl IntoIterator<Item = &'a str>) -> Option<Vec<FormChoice>> {
    let mut all = vec![FormChoice { value: String::new(), label: unset.to_owned() }];
    all.extend(values.into_iter().map(|v| FormChoice { value: v.to_owned(), label: v.to_owned() }));
    Some(all)
}

fn field(key: &str, label: &str, kind: FormFieldKind, hint: &str) -> FormField {
    FormField { key: key.to_owned(), label: label.to_owned(), kind, hint: hint.to_owned(), placeholder: None, choices: None, accepts: None }
}

fn with_placeholder(mut f: FormField, placeholder: &str) -> FormField {
    f.placeholder = Some(placeholder.to_owned());
    f
}

fn with_choices(mut f: FormField, choices: Option<Vec<FormChoice>>) -> FormField {
    f.choices = choices;
    f
}

/// The fields the form shows, in order. Everything else in a file is left to the Source tab.
pub static AGENT_FIELDS: LazyLock<Vec<FormField>> = LazyLock::new(|| {
    use FormFieldKind::*;
    let mut permission = with_choices(
        field("permissionMode", "Permission mode", Choice, "How the agent asks before it acts."),
        choices("As the session", AGENT_PERMISSION_MODES),
    );
    permission.accepts = Some(vec!["manual".to_owned()]);
    vec![
        with_placeholder(
            field("description", "Description", Long, "What the agent is for. Claude goes by this text when it decides whom to hand a task to."),
            "Reviews a diff for … Use after …",
        ),
        with_placeholder(field("tools", "Tools", List, "The tools the agent may use. Empty: every tool of the session."), "Read, Grep, Glob, Bash"),
        with_placeholder(field("disallowedTools", "Disallowed tools", List, "Tools taken away from what the agent would have."), "Write, Edit"),
        with_placeholder(
            field(
                "model",
                "Model",
                Text,
                "An alias (sonnet, opus, haiku, fable), a full model id, or inherit for the model of the session. Empty: Claude Code chooses.",
            ),
            "inherit",
        ),
        with_choices(field("effort", "Effort", Choice, "The effort the agent works with."), choices("As the session", EFFORT_LEVELS.iter().map(|e| e.as_str()))),
        permission,
        with_choices(
            field(
                "memory",
                "Memory",
                Choice,
                "A memory of its own that the agent keeps from one session to the next. user: in the account, for every project. project: in the project, shared with it. local: in the project, kept out of git.",
            ),
            choices("None", ["user", "project", "local"]),
        ),
        with_choices(
            field("isolation", "Isolation", Choice, "In a worktree the agent works on a copy of the repository of its own."),
            choices("None", ["worktree"]),
        ),
        with_choices(field("color", "Color", Choice, "The colour the agent shows in."), choices("Default", AGENT_COLORS)),
        with_placeholder(field("maxTurns", "Max turns", Number, "The agent stops after this many turns."), "No limit"),
        with_placeholder(field("skills", "Skills", Names, "Skills whose text the agent starts with."), "arch-review, unity-cli"),
        field("background", "Always in the background", Switch, "The agent runs as a background task whenever it is used."),
    ]
});

/// Keys the form or the manager handles itself, so they are not listed as "also in the file".
pub static AGENT_HANDLED_KEYS: LazyLock<Vec<String>> =
    LazyLock::new(|| std::iter::once("name".to_owned()).chain(AGENT_FIELDS.iter().map(|f| f.key.clone())).collect());

// ---------- what is wrong with an agent ----------

/// JavaScript's white space for `trim()` and `\s`: Unicode white space but U+0085, and the byte
/// order mark.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `s.trim()` in JavaScript.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `s.replace(/\s+/g, ' ')` in JavaScript.
pub fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if is_js_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// The name of an agent as Claude Code takes it: at most 256 characters, without a colon.
fn taken_name_problem(name: &str) -> Option<String> {
    if longer16(name, AGENT_LIMITS.name) {
        return Some(format!("The name has {} characters; Claude Code takes at most {}.", len16(name), AGENT_LIMITS.name));
    }
    if name.contains(':') {
        return Some(format!("The name \"{name}\" contains a colon, which Claude Code keeps for agents of plugins."));
    }
    None
}

/// `/^[1-9]\d*$/`
fn is_count(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && (b'1'..=b'9').contains(&bytes[0]) && bytes.iter().all(u8::is_ascii_digit)
}

fn issue(level: IssueLevel, message: String) -> AgentIssue {
    AgentIssue { level, message, clash: false }
}

/// What is wrong with an agent file, or worth knowing. An `error` keeps Claude Code from loading it.
pub fn inspect_agent(doc: &Doc<'_>) -> Vec<AgentIssue> {
    let mut issues = Vec::new();

    if let Some(error) = &doc.error {
        let at = error.line.map_or(String::new(), |line| format!(" (line {line})"));
        let said = if error.message.ends_with(['.', '!', '?']) { error.message.clone() } else { format!("{}.", error.message) };
        issues.push(issue(IssueLevel::Error, format!("The frontmatter cannot be read{at}: {said} Claude Code does not load the agent.")));
        return issues;
    }

    match name_of(doc) {
        None => issues.push(issue(
            IssueLevel::Error,
            "No name. Claude Code knows an agent by the \"name\" in its file, not by the name of the file, and does not load one without it.".to_owned(),
        )),
        Some(name) => {
            if let Some(problem) = taken_name_problem(&name) {
                issues.push(issue(IssueLevel::Error, problem));
            }
        }
    }
    if js_trim(&field_text(doc, "description", false)).is_empty() {
        issues.push(issue(IssueLevel::Error, "No description. Claude Code does not load an agent without one.".to_owned()));
    }

    for field in AGENT_FIELDS.iter() {
        let text = field_text(doc, &field.key, false);
        let value = js_trim(&text);
        if value.is_empty() {
            continue;
        }
        if field.kind == FormFieldKind::Choice {
            let offered = field.choices.as_deref().unwrap_or_default();
            let accepted = field.accepts.as_deref().unwrap_or_default();
            if !offered.iter().any(|c| c.value == value) && !accepted.iter().any(|a| a == value) {
                let known: Vec<&str> = offered.iter().map(|c| c.value.as_str()).filter(|v| !v.is_empty()).collect();
                issues.push(issue(
                    IssueLevel::Warning,
                    format!("\"{}: {value}\" is not a value Claude Code knows. It knows {}.", field.key, known.join(", ")),
                ));
            }
        }
        if field.kind == FormFieldKind::Number && !is_count(value) {
            issues.push(issue(IssueLevel::Warning, format!("\"{}: {value}\" is not a whole number above zero.", field.key)));
        }
    }

    if js_trim(doc.body).is_empty() {
        issues.push(issue(
            IssueLevel::Warning,
            "No system prompt. The text under the frontmatter is all the agent is told about its job.".to_owned(),
        ));
    } else if doc.lines > AGENT_LIMITS.lines {
        issues.push(issue(IssueLevel::Info, format!("The file has {} lines, all of which the agent reads every time it starts.", doc.lines)));
    }
    issues
}

/// Whether Claude Code can load the file, as far as its own text tells.
pub fn agent_loads(issues: &[AgentIssue]) -> bool {
    !issues.iter().any(|issue| issue.level == IssueLevel::Error && !issue.clash)
}

// ---------- names that get in each other's way ----------

/// An agent as far as [`agent_clashes`] needs it: where it is and the name in its file (empty
/// when the file gives none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedAgent<'a> {
    pub root_id: &'a str,
    pub file: &'a str,
    pub name: &'a str,
}

/// Notes about agents that carry the same name, by [`agent_key`]. Within one folder only one of
/// them is used; of a project's agent and a personal one Claude Code takes the project's (the other
/// way round than with skills). `accounts_of` names the accounts of a personal root, for the message.
pub fn agent_clashes(
    agents: &[NamedAgent<'_>],
    roots: &[AgentRoot],
    accounts_of: impl Fn(&AgentRoot) -> String,
) -> HashMap<String, Vec<AgentIssue>> {
    let mut result: HashMap<String, Vec<AgentIssue>> = HashMap::new();
    let mut add = |agent: &NamedAgent<'_>, level: IssueLevel, message: String| {
        result.entry(agent_key(agent.root_id, agent.file)).or_default().push(AgentIssue { level, message, clash: true });
    };
    let root_of = |id: &str| roots.iter().find(|r| r.id == id);
    let kind_of = |id: &str| root_of(id).map(|r| r.kind);
    let named: Vec<&NamedAgent<'_>> = agents.iter().filter(|a| !a.name.is_empty()).collect();

    for (at, agent) in named.iter().enumerate() {
        let twin = named.iter().enumerate().find(|(i, other)| *i != at && other.root_id == agent.root_id && other.name == agent.name);
        if let Some((_, twin)) = twin {
            add(agent, IssueLevel::Warning, format!("\"{}\" in the same folder has the same name. Only one of the two is used.", twin.file));
        }

        match kind_of(agent.root_id) {
            Some(SkillRootKind::Personal) => {
                let winner = named.iter().find(|other| kind_of(other.root_id) == Some(SkillRootKind::Project) && other.name == agent.name);
                if let Some(winner) = winner {
                    let folder = root_of(winner.root_id).and_then(|r| r.folder.as_deref()).filter(|f| !f.is_empty());
                    let place = folder.map_or(String::new(), |f| format!(" {f}"));
                    add(
                        agent,
                        IssueLevel::Warning,
                        format!(
                            "The project{place} has an agent of the same name (\"{}\"). In that project it is used instead of this one.",
                            winner.file
                        ),
                    );
                }
            }
            Some(SkillRootKind::Project) => {
                let loser = named.iter().find(|other| kind_of(other.root_id) == Some(SkillRootKind::Personal) && other.name == agent.name);
                if let Some(root) = loser.and_then(|l| root_of(l.root_id)) {
                    let accounts = accounts_of(root);
                    let whose = if accounts.is_empty() { String::new() } else { format!(" ({accounts})") };
                    add(agent, IssueLevel::Info, format!("Takes the place of the personal agent of the same name{whose} in this project."));
                }
            }
            None => {}
        }
    }
    result
}

// ---------- new agents ----------

/// The text of an agent that was just made.
pub fn new_agent_text(name: &str, description: &str) -> String {
    let description = match js_trim(description) {
        "" => "What this agent is for, and when Claude should hand work to it.",
        given => given,
    };
    let head = frontmatter_block(&[("name", YamlValue::from(name)), ("description", YamlValue::from(description))]);
    format!("{head}\nDescribe here how this agent works: what it looks at, what it may change, and what it hands back when it is done.\n")
}

#[cfg(test)]
mod tests {
    //! `test/agents.test.ts`.

    use super::*;
    use crate::frontmatter::parse_doc;

    fn lines(l: &[&str]) -> String {
        format!("{}\n", l.join("\n"))
    }

    // as the user's own agents are written
    fn real() -> String {
        lines(&[
            "---",
            "name: arch-reviewer",
            "description: Runs the arch-review skill and returns the report. Report-only, never edits code.",
            "model: opus",
            "effort: xhigh",
            "skills:",
            "  - arch-review",
            "tools: Read, Grep, Glob, Bash",
            "---",
            "",
            "You review the architecture.",
        ])
    }

    fn level(l: IssueLevel) -> &'static str {
        match l {
            IssueLevel::Error => "error",
            IssueLevel::Warning => "warning",
            IssueLevel::Info => "info",
        }
    }

    fn messages(text: &str) -> Vec<String> {
        inspect_agent(&parse_doc(text)).iter().map(|i| format!("{}: {}", level(i.level), i.message)).collect()
    }

    // ---------- inspectAgent ----------

    #[test]
    fn has_nothing_to_say_about_a_real_agent() {
        assert_eq!(messages(&real()), Vec::<String>::new());
        assert!(agent_loads(&inspect_agent(&parse_doc(&real()))));
    }

    #[test]
    fn says_that_claude_code_does_not_load_an_agent_without_a_name_or_a_description() {
        let text = lines(&["---", "model: opus", "---", "", "Text."]);
        let issues = inspect_agent(&parse_doc(&text));
        assert_eq!(issues.iter().map(|i| i.level).collect::<Vec<_>>(), [IssueLevel::Error, IssueLevel::Error]);
        assert!(issues[0].message.contains("No name."));
        assert!(issues[1].message.contains("No description."));
        assert!(!agent_loads(&issues));
        // a file without any frontmatter is the same case
        assert_eq!(messages("Just some notes.\n").iter().filter(|m| m.starts_with("error")).count(), 2);
    }

    #[test]
    fn reports_a_frontmatter_that_cannot_be_read_as_the_one_error() {
        let issues = inspect_agent(&parse_doc(&lines(&["---", "name: a", "description: Does: this", "---", "x"])));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].level, IssueLevel::Error);
        let m = &issues[0].message;
        assert!(m.starts_with("The frontmatter cannot be read (line 3): "), "{m}");
        assert!(m.ends_with(" Claude Code does not load the agent."), "{m}");
    }

    #[test]
    fn refuses_a_name_claude_code_would_not_take() {
        let named = |name: &str| lines(&["---", &format!("name: \"{name}\""), "description: d", "---", "x"]);
        assert_eq!(messages(&named("plugin:agent")), ["error: The name \"plugin:agent\" contains a colon, which Claude Code keeps for agents of plugins."]);
        assert_eq!(messages(&named(&"a".repeat(257)))[0], "error: The name has 257 characters; Claude Code takes at most 256.");
        // a name with capitals and spaces is unusual, and loads
        assert_eq!(messages(&named("Code Reviewer")), Vec::<String>::new());
    }

    #[test]
    fn names_a_value_that_is_none_of_the_choices_and_takes_the_ones_claude_code_also_knows() {
        let with_field = |line: &str| messages(&lines(&["---", "name: a", "description: d", line, "---", "x"]));
        assert_eq!(
            with_field("permissionMode: yolo"),
            ["warning: \"permissionMode: yolo\" is not a value Claude Code knows. It knows default, acceptEdits, auto, dontAsk, bypassPermissions, plan."]
        );
        assert_eq!(with_field("permissionMode: manual"), Vec::<String>::new());
        assert_eq!(with_field("memory: project"), Vec::<String>::new());
        assert!(with_field("color: teal")[0].contains("\"color: teal\" is not a value Claude Code knows."));
        assert_eq!(with_field("maxTurns: many"), ["warning: \"maxTurns: many\" is not a whole number above zero."]);
        assert_eq!(with_field("maxTurns: 0").len(), 1);
        assert_eq!(with_field("maxTurns: 25"), Vec::<String>::new());
        // a model may be any id
        assert_eq!(with_field("model: claude-opus-5-5"), Vec::<String>::new());
    }

    #[test]
    fn warns_about_an_agent_that_is_told_nothing() {
        assert_eq!(
            messages(&lines(&["---", "name: a", "description: d", "---"])),
            ["warning: No system prompt. The text under the frontmatter is all the agent is told about its job."]
        );
    }

    #[test]
    fn notes_a_long_system_prompt() {
        let body: Vec<String> = (0..600).map(|i| format!("Line {i}.")).collect();
        let text = format!("---\nname: a\ndescription: d\n---\n\n{}\n", body.join("\n"));
        assert_eq!(messages(&text), ["info: The file has 605 lines, all of which the agent reads every time it starts."]);
    }

    // ---------- agent names ----------

    #[test]
    fn asks_for_kebab_case_when_the_app_gives_the_name() {
        assert_eq!(agent_name_problem("code-reviewer"), None);
        assert_eq!(agent_name_problem("").as_deref(), Some("Give the agent a name."));
        assert_eq!(agent_name_problem("Code Reviewer").as_deref(), Some("Use lower-case letters, digits and hyphens only."));
        assert_eq!(agent_name_problem(&"a".repeat(65)).as_deref(), Some("A name has at most 64 characters."));
        assert_eq!(agent_name_problem("nul").as_deref(), Some("\"nul\" is a name Windows keeps for a device."));
        assert_eq!(agent_name_problem("com7").as_deref(), Some("\"com7\" is a name Windows keeps for a device."));
        assert_eq!(
            agent_name_problem("-a").as_deref(),
            Some("A name does not start or end with a hyphen, and has no two hyphens in a row.")
        );
        // a skills folder keeps this one for itself; an agent may have it
        assert_eq!(agent_name_problem("synced"), None);
        assert_eq!(agent_name_problem("console"), None);
    }

    #[test]
    fn knows_what_a_file_is_named_after() {
        assert_eq!(agent_stem("reviewer.md"), "reviewer");
        assert_eq!(agent_stem("team/qa/Tester.MD"), "Tester");
        assert_eq!(agent_stem("notes"), "notes");
        assert_eq!(agent_stem("ü.md"), "ü");
        assert_eq!(agent_stem("aü"), "aü");
    }

    #[test]
    fn writes_a_first_text_that_loads() {
        let text = new_agent_text("log-reader", "Reads logs: errors first.");
        let parsed = parse_doc(&text);
        assert_eq!(name_of(&parsed).as_deref(), Some("log-reader"));
        assert_eq!(field_text(&parsed, "description", false), "Reads logs: errors first.");
        assert_eq!(inspect_agent(&parsed), Vec::new());
        assert_eq!(inspect_agent(&parse_doc(&new_agent_text("x", ""))), Vec::new());
    }

    #[test]
    fn offers_every_field_once_and_only_choices_that_are_spelled_as_claude_code_spells_them() {
        let keys: Vec<&str> = AGENT_FIELDS.iter().map(|f| f.key.as_str()).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
        let by_key = |key: &str| AGENT_FIELDS.iter().find(|f| f.key == key).unwrap();
        let modes: Vec<&str> = by_key("permissionMode").choices.as_ref().unwrap().iter().map(|c| c.value.as_str()).collect();
        assert_eq!(modes, ["", "default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan"]);
        assert_eq!(by_key("skills").kind, FormFieldKind::Names);
        assert_eq!(by_key("maxTurns").kind, FormFieldKind::Number);
        let efforts: Vec<&str> = by_key("effort").choices.as_ref().unwrap().iter().map(|c| c.value.as_str()).collect();
        assert_eq!(efforts, ["", "low", "medium", "high", "xhigh", "max"]);
        assert_eq!(AGENT_HANDLED_KEYS[0], "name");
        assert_eq!(AGENT_HANDLED_KEYS.len(), AGENT_FIELDS.len() + 1);
    }

    // ---------- agentClashes ----------

    fn roots() -> Vec<AgentRoot> {
        vec![
            AgentRoot {
                id: "p-1".into(),
                kind: SkillRootKind::Personal,
                dir: "C:\\u\\.claude\\agents".into(),
                profile_ids: vec!["main".into(), "elif".into()],
                folder: None,
                exists: true,
                pinned: None,
            },
            AgentRoot {
                id: "j-1".into(),
                kind: SkillRootKind::Project,
                dir: "C:\\proj\\.claude\\agents".into(),
                profile_ids: vec![],
                folder: Some("C:\\proj".into()),
                exists: true,
                pinned: None,
            },
        ]
    }

    fn accounts(_: &AgentRoot) -> String {
        "Bilal, Elif".to_owned()
    }

    fn agent<'a>(root_id: &'a str, file: &'a str, name: &'a str) -> NamedAgent<'a> {
        NamedAgent { root_id, file, name }
    }

    #[test]
    fn says_that_the_project_agent_is_the_one_that_is_used() {
        let personal = agent("p-1", "reviewer.md", "reviewer");
        let project = agent("j-1", "team/reviewer.md", "reviewer");
        let clashes = agent_clashes(&[personal, project, agent("p-1", "other.md", "other")], &roots(), accounts);
        assert_eq!(
            clashes[&agent_key("p-1", "reviewer.md")],
            [AgentIssue {
                level: IssueLevel::Warning,
                clash: true,
                message: "The project C:\\proj has an agent of the same name (\"team/reviewer.md\"). In that project it is used instead of this one.".into()
            }]
        );
        assert_eq!(
            clashes[&agent_key("j-1", "team/reviewer.md")],
            [AgentIssue {
                level: IssueLevel::Info,
                clash: true,
                message: "Takes the place of the personal agent of the same name (Bilal, Elif) in this project.".into()
            }]
        );
        assert_eq!(clashes.len(), 2);
        // a clash is no reason for Claude Code not to load the file
        assert!(agent_loads(&clashes[&agent_key("p-1", "reviewer.md")]));
    }

    #[test]
    fn goes_by_the_name_in_the_file_not_by_the_file() {
        let clashes = agent_clashes(&[agent("p-1", "reviewer.md", "one"), agent("j-1", "reviewer.md", "two")], &roots(), accounts);
        assert!(clashes.is_empty());
    }

    #[test]
    fn warns_on_both_of_two_agents_in_one_folder_that_carry_the_same_name() {
        let clashes = agent_clashes(&[agent("p-1", "a.md", "same"), agent("p-1", "sub/b.md", "same")], &roots(), accounts);
        assert_eq!(clashes[&agent_key("p-1", "a.md")][0].message, "\"sub/b.md\" in the same folder has the same name. Only one of the two is used.");
        assert_eq!(clashes[&agent_key("p-1", "sub/b.md")][0].message, "\"a.md\" in the same folder has the same name. Only one of the two is used.");
    }

    #[test]
    fn leaves_files_without_a_name_out_of_it() {
        assert!(agent_clashes(&[agent("p-1", "README.md", ""), agent("j-1", "notes.md", "")], &roots(), accounts).is_empty());
    }

    #[test]
    fn collapses_white_space_as_javascript_does() {
        assert_eq!(collapse_spaces("a \n\t b\u{feff}c"), "a b c");
        assert_eq!(collapse_spaces("a\u{85}b"), "a\u{85}b");
    }
}
