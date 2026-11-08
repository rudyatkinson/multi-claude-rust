//! The non-frontmatter parts of `test/skillText.test.ts`: new skills and renames, `inspectSkill`,
//! names and `nameClashes` (the frontmatter parts are in `frontmatter/tests.rs`).

use super::*;
use crate::frontmatter::{field_text, parse_doc, set_body, set_field, FieldInput};
use crate::types::{EFFORT_LEVELS, UPLOAD_KEYS};

fn lines(l: &[&str]) -> String {
    format!("{}\n", l.join("\n"))
}

fn set(text: &str, key: &str, value: &str) -> String {
    set_field(text, key, FieldInput::from(value)).unwrap_or_else(|failure| panic!("{failure}"))
}

// as the user's own skills are written: a hint in brackets without quotes, an empty tools line
fn real() -> String {
    lines(&[
        "---",
        "name: arch-review",
        "argument-hint: [path | PR#]",
        "allowed-tools:",
        "description: Reviews the branch diff (or a given path/PR) against the architecture contract — layers, DI, naming.",
        "---",
        "",
        "# Arch review",
        "",
        "Do the review.",
    ])
}

// ---------- new skills and renames ----------

#[test]
fn writes_a_first_text_that_passes_every_check_but_the_missing_work() {
    let text = new_skill_text("review-pr", "Reviews a PR: use when asked.");
    assert_eq!(
        text,
        "---\nname: review-pr\ndescription: \"Reviews a PR: use when asked.\"\n---\n\n# Review Pr\n\nWrite here what Claude should do when this skill is used.\n"
    );
    assert_eq!(inspect_skill(&parse_doc(&text), "review-pr"), vec![]);
    assert!(field_text(&parse_doc(&new_skill_text("x", "")), "description", false).contains("What this skill does"));
}

#[test]
fn lets_the_name_in_the_file_follow_the_folder_when_it_was_the_folder_name() {
    let real = real();
    assert_eq!(rename_in_text(&real, "arch-review", "arch-check", false), real.replace("name: arch-review", "name: arch-check"));
    // a name of its own stays, unless a copy must not answer to the name of its original
    assert_eq!(rename_in_text(&real, "some-folder", "other", false), real);
    assert!(rename_in_text(&real, "some-folder", "other", true).contains("name: other\n"));
    // no name in the file: the folder is the name
    assert_eq!(rename_in_text("# text\n", "a", "b", true), "# text\n");
    // a frontmatter that cannot be read is left alone
    let broken = "---\nname: a\ndescription: Use when: it rains\n---\n";
    assert_eq!(rename_in_text(broken, "a", "b", true), broken);
}

#[test]
fn reads_who_may_start_the_skill() {
    let doc = parse_doc("---\ndisable-model-invocation: yes\nuser-invocable: false\n---\n");
    assert_eq!(invocation_of(&doc), Invocation { model_invocable: false, user_invocable: false });
    assert_eq!(invocation_of(&parse_doc("# text\n")), Invocation { model_invocable: true, user_invocable: true });
}

// ---------- inspectSkill ----------

fn messages(text: &str, dir: &str, scope: IssueScope) -> Vec<String> {
    let level = |l: IssueLevel| match l {
        IssueLevel::Error => "error",
        IssueLevel::Warning => "warning",
        IssueLevel::Info => "info",
    };
    inspect_skill(&parse_doc(text), dir).into_iter().filter(|i| i.scope == scope).map(|i| format!("{}: {}", level(i.level), i.message)).collect()
}

#[test]
fn has_nothing_to_say_to_claude_code_about_the_real_file_and_names_what_claude_ai_would_refuse() {
    assert_eq!(messages(&real(), "arch-review", IssueScope::Claude), Vec::<String>::new());
    assert_eq!(
        messages(&real(), "arch-review", IssueScope::Upload),
        ["warning: claude.ai does not accept this field: argument-hint. Claude Code uses them."]
    );
}

#[test]
fn reports_a_frontmatter_that_cannot_be_read_as_an_error_and_nothing_else() {
    let issues = inspect_skill(&parse_doc("---\nname: x\ndescription: Use when: it rains\n---\n"), "x");
    assert_eq!(issues.len(), 1);
    assert_eq!((issues[0].level, issues[0].scope), (IssueLevel::Error, IssueScope::Claude));
    assert!(issues[0].message.contains("(line 3)"), "{}", issues[0].message);
    assert!(issues[0].message.contains("without any of its fields"));
}

#[test]
fn warns_about_a_missing_or_overlong_description_and_a_very_long_file() {
    assert!(messages("---\nname: x\n---\n# Title\n", "x", IssueScope::Claude)[0].contains("No description"));
    let long = set(&real(), "description", &"x".repeat(1600));
    assert!(messages(&long, "arch-review", IssueScope::Claude)[0].contains("1600 characters"));
    assert!(messages(&long, "arch-review", IssueScope::Upload).join(" ").contains("at most 1024"));
    let tall = set_body(&real(), &"line\n".repeat(600));
    assert!(messages(&tall, "arch-review", IssueScope::Claude)[0].contains("info: SKILL.md has 607 lines"));
}

#[test]
fn checks_the_name_only_for_the_upload() {
    assert_eq!(messages(&real(), "other-folder", IssueScope::Claude), Vec::<String>::new());
    assert!(messages(&real(), "other-folder", IssueScope::Upload)[0].contains("differs from the folder"));
    assert!(messages(&set(&real(), "name", "Arch Review"), "arch-review", IssueScope::Upload)[0].contains("does not pass on claude.ai"));
    assert!(messages("# text\n", "x", IssueScope::Upload)[0].contains("No name in the file"));
    assert!(messages(&set(&real(), "description", "a <b> c"), "arch-review", IssueScope::Upload).join(" ").contains("< or >"));
}

#[test]
fn says_several_fields_and_an_overlong_compatibility() {
    let text = "---\nname: x\ndescription: d\nmodel: opus\ncontext: fork\n---\n";
    assert_eq!(messages(text, "x", IssueScope::Upload), ["warning: claude.ai does not accept these fields: model, context. Claude Code uses them."]);
    let text = set("---\nname: x\ndescription: d\n---\n", "compatibility", &"c".repeat(501));
    assert_eq!(messages(&text, "x", IssueScope::Upload), ["warning: \"compatibility\" is longer than 500 characters."]);
}

// ---------- names ----------

#[test]
fn accepts_kebab_case_and_says_what_is_wrong_with_anything_else() {
    assert_eq!(new_name_problem("review-pr-2"), None);
    let problem = |name: &str| new_name_problem(name).unwrap_or_default();
    assert!(problem("").contains("Give the skill a name"));
    assert!(problem("Review").contains("lower-case"));
    assert!(problem("a b").contains("lower-case"));
    assert!(problem("a/b").contains("lower-case"));
    assert!(problem("..").contains("lower-case"));
    assert!(problem("-a").contains("hyphen"));
    assert!(problem("a--b").contains("hyphen"));
    assert!(problem(&"x".repeat(65)).contains("at most 64"));
    assert!(problem("synced").contains("Claude Code keeps"));
    assert!(problem("nul").contains("Windows keeps"));
    assert!(problem("com1").contains("Windows keeps"));
    assert_eq!(new_name_problem("com10"), None);
}

#[test]
fn knows_the_folders_that_are_not_skills() {
    for name in ["synced", "Synced", ".trash", ".mc-new-1", "anthropic-skills"] {
        assert!(is_reserved_skill_dir(name), "{name}");
    }
    assert!(!is_reserved_skill_dir("my-skill"));
}

#[test]
fn makes_a_name_from_a_title() {
    assert_eq!(slugify_skill_name("  Review PR (quick) "), "review-pr-quick");
    assert_eq!(slugify_skill_name("İnceleme Işık çğş"), "inceleme-isik-cgs");
    assert_eq!(slugify_skill_name("!!!"), "");
    assert_eq!(slugify_skill_name(&"x".repeat(80)).len(), 64);
    // a cut that ends on a hyphen loses it
    assert_eq!(slugify_skill_name(&format!("{} b", "a".repeat(63))), "a".repeat(63));
}

#[test]
fn finds_a_free_name() {
    let none: [&str; 0] = [];
    assert_eq!(free_skill_name("review", &none), "review");
    assert_eq!(free_skill_name("review", &["Review", "review-2"]), "review-3");
    assert_eq!(free_skill_name(&"x".repeat(64), &["x".repeat(64)]), format!("{}-2", "x".repeat(62)));
}

// ---------- nameClashes ----------

fn roots() -> Vec<SkillRoot> {
    vec![
        SkillRoot {
            id: "p".into(),
            kind: SkillRootKind::Personal,
            dir: "/home/.claude/skills".into(),
            profile_ids: vec!["main".into(), "work".into()],
            folder: None,
            exists: true,
            pinned: None,
        },
        SkillRoot {
            id: "j".into(),
            kind: SkillRootKind::Project,
            dir: "/code/app/.claude/skills".into(),
            profile_ids: vec![],
            folder: Some("/code/app".into()),
            exists: true,
            pinned: None,
        },
    ]
}

fn clashes(skills: &[(&str, &str, &str)]) -> BTreeMap<String, Vec<String>> {
    let named: Vec<Named<'_>> = skills.iter().map(|&(root_id, dir_name, name)| Named { root_id, dir_name, name }).collect();
    name_clashes(&named, &roots(), |_| "Main, Work".to_owned()).into_iter().map(|(key, issues)| (key, issues.into_iter().map(|i| i.message).collect())).collect()
}

#[test]
fn says_which_project_skill_a_personal_one_hides() {
    let result = clashes(&[("p", "review", "review"), ("j", "review", "review"), ("j", "deploy", "deploy")]);
    assert_eq!(result.keys().collect::<Vec<_>>(), ["j/review"]);
    assert_eq!(result["j/review"][0], "The personal skill \"review\" (Main, Work) has the same name and is used instead of this one.");
    let issue = &name_clashes(&[Named { root_id: "p", dir_name: "a", name: "a" }, Named { root_id: "j", dir_name: "a", name: "a" }], &roots(), |_| String::new())["j/a"][0];
    assert_eq!((issue.level, issue.scope, issue.clash), (IssueLevel::Warning, IssueScope::Claude, true));
    // without account names there are no brackets
    assert_eq!(issue.message, "The personal skill \"a\" has the same name and is used instead of this one.");
}

#[test]
fn goes_by_the_folder_name_and_by_the_name_in_the_file() {
    assert_eq!(clashes(&[("p", "folder-a", "review"), ("j", "review", "review")]).keys().collect::<Vec<_>>(), ["j/review"]);
    assert_eq!(clashes(&[("p", "Review", "Review"), ("j", "x", "review")]).keys().collect::<Vec<_>>(), ["j/x"]);
    assert!(clashes(&[("p", "a", "a"), ("j", "b", "b")]).is_empty());
}

#[test]
fn warns_on_both_of_two_skills_in_one_folder_that_answer_to_the_same_name() {
    let result = clashes(&[("p", "one", "shared"), ("p", "two", "shared")]);
    assert_eq!(result.keys().collect::<Vec<_>>(), ["p/one", "p/two"]);
    assert!(result["p/one"][0].contains("\"two\" in the same folder"));
}

// ---------- the fields ----------

#[test]
fn offers_the_effort_levels_and_knows_the_keys_it_handles() {
    let effort = SKILL_FIELDS.iter().find(|f| f.key == "effort").unwrap();
    let offered: Vec<&str> = effort.choices.iter().skip(1).map(|(value, _)| *value).collect();
    assert_eq!(offered, EFFORT_LEVELS.iter().map(|e| e.as_str()).collect::<Vec<_>>());
    assert_eq!(effort.choices[0], ("", "As the session"));
    let keys = handled_keys();
    assert_eq!(keys[0], "name");
    assert!(keys.contains(&"paths") && keys.contains(&"disable-model-invocation") && keys.contains(&"user-invocable"));
    // every field the upload accepts that the form shows is handled
    assert!(UPLOAD_KEYS.iter().filter(|k| SKILL_FIELDS.iter().any(|f| f.key == **k)).all(|k| keys.contains(k)));
}

#[test]
fn writes_the_fields_as_the_renderer_reads_them() {
    let fields = serde_json::to_value(skill_fields()).unwrap();
    assert_eq!(fields[0]["key"], "description");
    assert_eq!(fields[0]["kind"], "long");
    assert!(fields[0].get("choices").is_none());
    assert_eq!(fields[7]["choices"][1], serde_json::json!({ "value": "fork", "label": "In a subagent" }));
    assert!(fields[1].get("placeholder").is_none());
}

#[test]
fn names_a_skill_by_its_root_and_folder() {
    assert_eq!(skill_key(&SkillRef { root_id: "p-1".into(), dir_name: "review".into() }), "p-1/review");
}
