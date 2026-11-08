//! `test/frontmatter.test.ts`, the frontmatter parts of `test/skillText.test.ts` (split, parse,
//! setField, setBody) and of `test/agents.test.ts`, plus checks of the YAML writer.

use super::*;

fn lines(l: &[&str]) -> String {
    format!("{}\n", l.join("\n"))
}

fn set(text: &str, key: &str, value: impl Into<FieldInput>) -> String {
    match set_field(text, key, value.into()) {
        Ok(text) => text,
        Err(failure) => panic!("{failure}"),
    }
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

// ---------- test/frontmatter.test.ts ----------

// as a subagent is written: a number, a list in two spellings, a field the form does not know
fn agent() -> String {
    lines(&[
        "---",
        "name: code-reviewer",
        "description: Reviews a diff for bugs.",
        "tools: Read, Grep, Glob",
        "maxTurns: 20",
        "# what it brings along",
        "skills:",
        "  - arch-review",
        "  - unity-cli",
        "mcpServers:",
        "  - github",
        "---",
        "",
        "You review code.",
    ])
}

#[test]
fn numbers_write_a_number_without_quotes_and_read_it_back() {
    let agent = agent();
    let next = set(&agent, "maxTurns", 35u32);
    assert!(next.contains("\nmaxTurns: 35\n"));
    assert_eq!(field_text(&parse_doc(&next), "maxTurns", false), "35");
    // nothing else moved
    assert_eq!(next.replace("maxTurns: 35", "maxTurns: 20"), agent);
}

#[test]
fn numbers_are_added_to_a_file_that_had_none() {
    let next = set(&lines(&["---", "name: a", "---", "", "Text."]), "maxTurns", 5u32);
    assert_eq!(next, lines(&["---", "name: a", "maxTurns: 5", "---", "", "Text."]));
}

#[test]
fn numbers_typed_as_text_stay_a_text() {
    // the form's number field hands over a number; a text of digits is quoted, as before
    assert!(set(&agent(), "maxTurns", "35").contains("maxTurns: \"35\""));
}

#[test]
fn numbers_refuse_what_is_not_a_number() {
    assert_eq!(set_field(&agent(), "maxTurns", FieldInput::Num(f64::NAN)), Err(SetFieldFailure("\"maxTurns\" needs a number.".to_owned())));
    assert!(set_field(&agent(), "maxTurns", FieldInput::Num(f64::INFINITY)).is_err());
}

#[test]
fn lists_write_several_names_as_a_yaml_list_and_change_only_those_lines() {
    let next = set(&agent(), "skills", strs(&["arch-review", "implement-feature", "unity-cli"]));
    assert!(next.contains(&lines(&["skills:", "  - arch-review", "  - implement-feature", "  - unity-cli", "mcpServers:"])));
    assert_eq!(field_items(&parse_doc(&next), "skills"), strs(&["arch-review", "implement-feature", "unity-cli"]));
    // the comment above the field and the field after it stay
    assert!(next.contains("# what it brings along\nskills:"));
    assert!(next.contains(&lines(&["mcpServers:", "  - github", "---"])));
}

#[test]
fn lists_quote_an_entry_yaml_would_read_as_something_else() {
    let next = set(&agent(), "skills", strs(&["true", "12", "a: b"]));
    assert_eq!(field_items(&parse_doc(&next), "skills"), strs(&["true", "12", "a: b"]));
    assert!(next.contains("skills:\n  - \"true\"\n  - \"12\"\n  - \"a: b\"\n"));
}

#[test]
fn lists_take_the_field_out_when_empty() {
    let next = set(&agent(), "skills", Vec::<String>::new());
    assert_eq!(parse_doc(&next).keys, strs(&["name", "description", "tools", "maxTurns", "mcpServers"]));
    assert_eq!(set(&next, "skills", Vec::<String>::new()), next);
}

#[test]
fn lists_are_added_to_a_file_that_had_none() {
    let next = set(&lines(&["---", "name: a", "---"]), "skills", strs(&["one"]));
    assert_eq!(next, lines(&["---", "name: a", "skills:", "  - one", "---"]));
}

#[test]
fn lists_read_their_entries_however_they_are_written() {
    let agent = agent();
    let parsed = parse_doc(&agent);
    assert_eq!(field_items(&parsed, "skills"), strs(&["arch-review", "unity-cli"]));
    assert_eq!(field_items(&parsed, "tools"), strs(&["Read", "Grep", "Glob"]));
    let bracketed = lines(&["---", "tools: [Read, Bash]", "---"]);
    assert_eq!(field_items(&parse_doc(&bracketed), "tools"), strs(&["Read", "Bash"]));
    assert_eq!(field_items(&parsed, "nothing"), Vec::<String>::new());
    assert_eq!(split_items(" a, ,b ,"), strs(&["a", "b"]));
}

#[test]
fn frontmatter_block_writes_the_fields_of_a_new_file_so_that_they_read_back() {
    let block = frontmatter_block(&[("name", "x".into()), ("description", "Says: use it. #1".into()), ("maxTurns", 3u32.into())]);
    assert_eq!(block, "---\nname: x\ndescription: \"Says: use it. #1\"\nmaxTurns: 3\n---\n");
    let text = format!("{block}\nBody.\n");
    let parsed = parse_doc(&text);
    assert_eq!(parsed.error, None);
    assert_eq!(field_text(&parsed, "description", false), "Says: use it. #1");
    assert_eq!(field_text(&parsed, "maxTurns", false), "3");
    assert_eq!(parsed.body, "Body.\n");
}

#[test]
fn frontmatter_block_writes_a_nested_map_in_order() {
    // as newMemoryText writes a memory
    let block = frontmatter_block(&[
        ("name", "user-role".into()),
        ("description", "The user is a game developer.".into()),
        ("metadata", YamlValue::Map(vec![("node_type".into(), "memory".into()), ("type".into(), "user".into()), ("modified".into(), "2026-10-10".into())])),
    ]);
    assert_eq!(
        block,
        "---\nname: user-role\ndescription: The user is a game developer.\nmetadata:\n  node_type: memory\n  type: user\n  modified: 2026-10-10\n---\n"
    );
    assert_eq!(parse_doc(&block).error, None);
    assert_eq!(frontmatter_block(&[]), "---\n{}\n---\n");
}

// ---------- test/skillText.test.ts: as the user's own skills are written ----------

// a hint in brackets without quotes, an empty tools line
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

fn rich() -> String {
    lines(&[
        "---",
        "# who wrote this",
        "name: unity-cli  # the command",
        "allowed-tools:",
        "- Read",
        "- Bash(git *, gh pr *)",
        "",
        "# what it is for",
        "description: >-",
        "  Use when driving Unity",
        "  from the terminal.",
        "hooks:",
        "  Stop:",
        "    - hooks:",
        "        - type: command",
        "          command: echo done",
        "user-invocable: no",
        "---",
        "Body right under the fence.",
    ])
}

#[test]
fn split_finds_the_frontmatter_between_the_fences() {
    let real = real();
    let parts = split_doc(&real);
    assert_eq!(
        parts.frontmatter,
        Some(lines(&[
            "name: arch-review",
            "argument-hint: [path | PR#]",
            "allowed-tools:",
            "description: Reviews the branch diff (or a given path/PR) against the architecture contract — layers, DI, naming."
        ]))
        .as_deref()
    );
    assert_eq!(&real[parts.body_start..], "\n# Arch review\n\nDo the review.\n");
    assert!(!parts.unclosed);
}

#[test]
fn split_reads_an_empty_frontmatter_and_fences_with_spaces_after_them() {
    let parts = split_doc("---\n---\nbody");
    assert_eq!((parts.frontmatter, parts.body_start), (Some(""), 8));
    assert_eq!(split_doc("--- \nname: x\n---\t\nbody").frontmatter, Some("name: x\n"));
    assert_eq!(split_doc("---\nname: x\n---").frontmatter, Some("name: x\n"));
}

#[test]
fn split_takes_a_file_that_does_not_open_with_a_fence_as_text_only() {
    let parts = split_doc("# Title\n---\nname: x\n---\n");
    assert_eq!((parts.frontmatter, parts.body_start, parts.unclosed), (None, 0, false));
    assert_eq!(split_doc("\n---\nname: x\n---\n").frontmatter, None);
    assert_eq!(split_doc("").frontmatter, None);
}

#[test]
fn split_notices_a_frontmatter_that_is_never_closed() {
    let parts = split_doc("---\nname: x\n\n# Title\n");
    assert_eq!((parts.frontmatter, parts.unclosed), (None, true));
    assert!(split_doc("---").unclosed);
    // a fence that is indented belongs to a value
    assert!(split_doc("---\ndescription: |\n  ---\n  more\n").unclosed);
}

#[test]
fn parse_reads_the_fields_as_the_form_shows_them() {
    let real = real();
    let parsed = parse_doc(&real);
    assert_eq!(parsed.error, None);
    assert_eq!(parsed.keys, strs(&["name", "argument-hint", "allowed-tools", "description"]));
    assert_eq!(name_of(&parsed).as_deref(), Some("arch-review"));
    // brackets without quotes are a list to YAML; the form shows what the author wrote
    assert_eq!(field_text(&parsed, "argument-hint", false), "[path | PR#]");
    assert_eq!(field_text(&parsed, "allowed-tools", true), "");
    assert!(field_text(&parsed, "description", false).contains("architecture contract — layers"));
    assert_eq!(field_text(&parsed, "model", false), "");
    assert_eq!(parsed.body, "# Arch review\n\nDo the review.\n");
    assert_eq!(parsed.lines, 10);
}

#[test]
fn parse_reads_lists_folded_text_comments_and_switches() {
    let rich = rich();
    let parsed = parse_doc(&rich);
    assert_eq!(parsed.error, None);
    assert_eq!(parsed.keys, strs(&["name", "allowed-tools", "description", "hooks", "user-invocable"]));
    assert_eq!(name_of(&parsed).as_deref(), Some("unity-cli"));
    assert_eq!(field_text(&parsed, "allowed-tools", true), "Read, Bash(git *, gh pr *)");
    assert_eq!(field_text(&parsed, "description", false), "Use when driving Unity from the terminal.");
    assert!(!field_bool(&parsed, "user-invocable", true));
    assert!(field_bool(&parsed, "disable-model-invocation", true));
    assert_eq!(parsed.body, "Body right under the fence.\n");
}

#[test]
fn parse_keeps_the_values_as_they_are_written() {
    let rich = rich();
    let parsed = parse_doc(&rich);
    let source = |key: &str| parsed.values[key].source.clone();
    assert_eq!(source("name"), "unity-cli");
    assert_eq!(source("allowed-tools"), "- Read\n- Bash(git *, gh pr *)");
    assert_eq!(source("description"), ">-\n  Use when driving Unity\n  from the terminal.");
    assert_eq!(source("hooks"), "Stop:\n    - hooks:\n        - type: command\n          command: echo done");
    assert_eq!(source("user-invocable"), "no");
    let text = lines(&["---", "a: [1, 2] # c", "b: 'it''s' # c", "c: \"x # y\"", "d: true", "e: 0x1F", "f:", "  - 1.50", "  - null", "---"]);
    let parsed = parse_doc(&text);
    assert_eq!(parsed.values["a"], Value { source: "[1, 2]".into(), string: None, items: Some(strs(&["1", "2"])) });
    assert_eq!(parsed.values["b"], Value { source: "'it''s'".into(), string: Some("it's".into()), items: None });
    assert_eq!(parsed.values["c"].source, "\"x # y\"");
    assert_eq!(parsed.values["d"], Value { source: "true".into(), string: None, items: None });
    assert_eq!(parsed.values["e"].source, "0x1F");
    // a null entry makes it something other than a list of plain values
    assert_eq!(parsed.values["f"].items, None);
    assert_eq!(field_items(&parse_doc(&lines(&["---", "n: [0x1F, 1.50, .inf, TRUE]", "---"])), "n"), strs(&["31", "1.5", "Infinity", "true"]));
}

#[test]
fn parse_says_why_the_fields_cannot_be_read_with_the_line_in_the_file() {
    let colon = parse_doc("---\nname: x\ndescription: Use when: it rains\n---\n");
    let error = colon.error.unwrap();
    assert_eq!(error.line, Some(3));
    assert_eq!(error.message, "A value contains a colon followed by a space, and has to stand in quotes for that.");
    assert_eq!(parse_doc("---\nname: a\nname: b\n---\n").error, Some(FmError { message: "A field is written twice.".into(), line: Some(3) }));
    let open = parse_doc("---\nname: x\ndescription: \"open\n---\n").error.unwrap();
    assert!(!open.message.contains("at line") && !open.message.contains(" byte "), "{}", open.message);
    assert!(parse_doc("---\n- a\n- b\n---\n").error.unwrap().message.contains("not a list of"));
    assert!(parse_doc("---\n{name: x}\n---\n").error.unwrap().message.contains("braces"));
    assert!(parse_doc("---\nname: x\n").error.unwrap().message.contains("never closed"));
}

#[test]
fn parse_names_tabs_and_second_documents() {
    let tab = parse_doc("---\nname: x\n\tdescription: y\n---\n").error.unwrap();
    assert_eq!(tab, FmError { message: "A line is indented with a tab; YAML wants spaces.".into(), line: Some(3) });
    let nested = parse_doc("---\nhooks:\n\tStop: x\n---\n").error.unwrap();
    assert_eq!(nested.line, Some(3));
    assert_eq!(nested.message, "A line is indented with a tab; YAML wants spaces.");
    let docs = parse_doc("---\nname: x\n...\nfoo: bar\n---\n").error.unwrap();
    assert_eq!(docs.line, Some(4));
    // a key written twice in a nested map counts too
    assert!(parse_doc("---\nhooks:\n  Stop: a\n  Stop: b\n---\n").error.is_some());
    // what was read up to the error still counts
    let broken = parse_doc("---\nname: x\ndescription: Use when: it rains\n---\n");
    assert_eq!(name_of(&broken).as_deref(), Some("x"));
}

#[test]
fn parse_reads_a_file_without_frontmatter_as_text() {
    let parsed = parse_doc("# Just text\n\nmore\n");
    assert_eq!((parsed.error.clone(), parsed.keys.len(), parsed.body, parsed.lines), (None, 0, "# Just text\n\nmore\n", 3));
    assert_eq!(name_of(&parsed), None);
    assert_eq!(first_body_line(parsed.body), "Just text");
    let empty = parse_doc("");
    assert_eq!((empty.keys.len(), empty.body, empty.lines), (0, "", 0));
    assert_eq!(first_body_line("\n  \n## Two  \nthree"), "Two");
    assert_eq!(first_body_line("  # not a heading\n"), "# not a heading");
    assert_eq!(first_body_line(""), "");
}

#[test]
fn parse_reads_yes_and_no_on_and_off_1_and_0_as_switches() {
    assert_eq!(["true", "Yes", "ON", "1"].map(as_bool), [Some(true); 4]);
    assert_eq!(["false", "no", "Off", "0"].map(as_bool), [Some(false); 4]);
    assert_eq!(as_bool("maybe"), None);
    let text = "---\ndisable-model-invocation: yes\nuser-invocable: maybe\n---\n";
    let parsed = parse_doc(text);
    assert!(field_bool(&parsed, "disable-model-invocation", false));
    assert!(field_bool(&parsed, "user-invocable", true));
}

#[test]
fn set_changes_the_lines_of_one_field_and_nothing_else() {
    let real = real();
    let next = set(&real, "description", "Short now.");
    let expected: String = real
        .split_inclusive('\n')
        .map(|line| if line.starts_with("description: ") { "description: Short now.\n" } else { line })
        .collect();
    assert_eq!(next, expected);
    // the bracket hint and the empty tools line stand as they were
    assert!(next.contains("argument-hint: [path | PR#]\nallowed-tools:\n"));
}

#[test]
fn set_keeps_comments_order_and_fields_the_form_does_not_know() {
    let rich = rich();
    let next = set(&rich, "description", "One line.");
    assert_eq!(next, rich.replace("description: >-\n  Use when driving Unity\n  from the terminal.\n", "description: One line.\n"));
    assert!(next.contains("\n\n# what it is for\ndescription: One line.\nhooks:\n  Stop:"));
    let tools = set(&rich, "allowed-tools", "Read, Grep");
    assert!(tools.contains("name: unity-cli  # the command\nallowed-tools: Read, Grep\n\n# what it is for\n"));
    assert!(tools.contains("          command: echo done\nuser-invocable: no\n---\nBody right under the fence.\n"));
}

#[test]
fn set_writes_a_value_as_typed_when_yaml_reads_it_the_same_way_else_in_quotes() {
    let real = real();
    assert!(set(&real, "argument-hint", "[file | diff]").contains("\nargument-hint: [file | diff]\n"));
    assert!(set(&real, "description", "Use when: it rains").contains("\ndescription: \"Use when: it rains\"\n"));
    assert!(set(&real, "description", "ends with a hash # not a comment").contains("description: \"ends with a hash # not a comment\""));
    assert!(set(&real, "description", "true").contains("\ndescription: \"true\"\n"));
    assert!(set(&real, "model", "123").contains("\nmodel: \"123\"\n"));
    assert!(set(&real, "description", "it's fine").contains("\ndescription: it's fine\n"));
    assert!(set(&real, "description", "Türkçe açıklama — çalışır").contains("\ndescription: Türkçe açıklama — çalışır\n"));
}

#[test]
fn set_reads_back_exactly_what_was_written() {
    let real = real();
    for value in [
        "plain",
        "[a | b]",
        "a: b",
        "- not a list",
        "\"quoted\"",
        "'single'",
        "x # y",
        "{braces}",
        "@at",
        "*star",
        "!bang",
        "% percent",
        "back\\slash",
        "50%",
        "a, b, Bash(git *, gh pr *)",
    ] {
        assert_eq!(field_text(&parse_doc(&set(&real, "description", value)), "description", false), value, "{value}");
    }
}

#[test]
fn set_writes_several_lines_as_a_block_and_reads_them_back() {
    let next = set(&real(), "description", "First line.\nSecond line.");
    assert!(next.contains("\ndescription: |-\n  First line.\n  Second line.\n---\n"));
    assert_eq!(field_text(&parse_doc(&next), "description", false), "First line.\nSecond line.");
}

#[test]
fn set_adds_a_field_at_the_end_and_takes_one_out() {
    let real = real();
    let rich = rich();
    let added = set(&real, "model", "opus");
    assert!(added.contains("naming.\nmodel: opus\n---\n"));
    assert_eq!(parse_doc(&added).keys, strs(&["name", "argument-hint", "allowed-tools", "description", "model"]));
    assert_eq!(set(&added, "model", ""), real);
    assert_eq!(set(&added, "model", FieldInput::Remove), real);
    // taking out what is not there changes nothing
    assert_eq!(set(&real, "effort", FieldInput::Remove), real);
    assert_eq!(set(&rich, "allowed-tools", FieldInput::Remove), rich.replace("allowed-tools:\n- Read\n- Bash(git *, gh pr *)\n", ""));
}

#[test]
fn set_sets_and_clears_the_switches() {
    let real = real();
    let off = set(&real, "disable-model-invocation", true);
    assert!(off.contains("\ndisable-model-invocation: true\n---\n"));
    assert!(field_bool(&parse_doc(&off), "disable-model-invocation", false));
    assert_eq!(set(&off, "disable-model-invocation", FieldInput::Remove), real);
    assert!(set(&rich(), "user-invocable", true).contains("\nuser-invocable: true\n---\n"));
}

#[test]
fn set_gives_a_file_without_frontmatter_one() {
    assert_eq!(set("# Title\n\ntext\n", "description", "Does things."), "---\ndescription: Does things.\n---\n\n# Title\n\ntext\n");
    assert_eq!(set("", "name", "x"), "---\nname: x\n---\n");
    assert_eq!(set("---\n---\nbody\n", "name", "x"), "---\nname: x\n---\nbody\n");
    assert_eq!(set("# Title\n", "description", FieldInput::Remove), "# Title\n");
}

#[test]
fn set_refuses_instead_of_guessing_when_the_frontmatter_is_broken() {
    let broken = "---\nname: x\ndescription: Use when: it rains\n---\n";
    assert_eq!(set_field(broken, "model", "opus".into()), Err(SetFieldFailure("The frontmatter has a problem. Fix it in the Source tab first.".into())));
    assert!(set_field("---\nname: x\n", "model", "opus".into()).is_err());
    assert_eq!(set_field(&real(), "not a key", "x".into()), Err(SetFieldFailure("\"not a key\" cannot be set from the form.".into())));
}

#[test]
fn set_changes_only_the_first_of_a_field_that_is_written_with_quotes_around_its_key() {
    let quoted = "---\n\"name\": x\n'description': old\n---\n";
    assert_eq!(set(quoted, "description", "new"), "---\n\"name\": x\ndescription: new\n---\n");
}

#[test]
fn set_works_under_a_closing_fence_without_a_line_break_and_turns_a_text_into_a_list() {
    assert_eq!(set("---\nname: x\n---", "model", "opus"), "---\nname: x\nmodel: opus\n---");
    let listed = set(&real(), "description", strs(&["a"]));
    assert!(listed.contains("\ndescription:\n  - a\n---\n"));
    assert_eq!(parse_doc(&listed).values["description"].items, Some(strs(&["a"])));
}

#[test]
fn set_body_replaces_the_text_under_the_frontmatter_and_keeps_the_empty_line_after_the_fence() {
    let real = real();
    let rich = rich();
    assert_eq!(set_body(&real, "New body.\n"), real.replace("# Arch review\n\nDo the review.\n", "New body.\n"));
    assert_eq!(set_body(&rich, "Other.\n"), rich.replace("Body right under the fence.\n", "Other.\n"));
}

#[test]
fn set_body_is_the_same_text_when_the_body_is_the_same() {
    for text in [real(), rich(), "# no frontmatter\n".into(), "---\nname: x\n---".into(), "---\nname: x\n---\n".into(), "---\nname: x\n---\n\n".into()] {
        let body = parse_doc(&text).body.to_owned();
        assert_eq!(set_body(&text, &body), text);
    }
}

#[test]
fn set_body_starts_a_body_under_a_frontmatter_that_had_none() {
    assert_eq!(set_body("---\nname: x\n---\n", "Hello\n"), "---\nname: x\n---\n\nHello\n");
    assert_eq!(set_body("---\nname: x\n---", "Hello\n"), "---\nname: x\n---\n\nHello\n");
    assert_eq!(set_body("plain\n", "other\n"), "other\n");
}

// ---------- test/agents.test.ts: the frontmatter of a real agent ----------

#[test]
fn agents_read_a_real_agent_and_say_where_its_frontmatter_breaks() {
    let text = lines(&[
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
    ]);
    let parsed = parse_doc(&text);
    assert_eq!(parsed.error, None);
    assert_eq!(name_of(&parsed).as_deref(), Some("arch-reviewer"));
    assert_eq!(field_items(&parsed, "skills"), strs(&["arch-review"]));
    assert_eq!(first_body_line(parsed.body), "You review the architecture.");
    let broken = lines(&["---", "name: a", "description: Does: this", "---", "x"]);
    assert_eq!(parse_doc(&broken).error.map(|e| e.line), Some(Some(3)));
    let quoted_name = lines(&["---", "name: \"Code Reviewer\"", "description: d", "---", "x"]);
    assert_eq!(name_of(&parse_doc(&quoted_name)).as_deref(), Some("Code Reviewer"));
    assert_eq!(field_text(&parse_doc(&lines(&["---", "maxTurns: 25", "---"])), "maxTurns", false), "25");
}

// ---------- the YAML writer ----------

#[test]
fn stringify_quotes_like_the_yaml_package() {
    let pair = |v: &str| stringify::stringify_pair("k", &YamlValue::Str(v.to_owned()), "");
    assert_eq!(pair("plain text"), "k: plain text");
    assert_eq!(pair(""), "k: \"\"");
    assert_eq!(pair("null"), "k: \"null\"");
    assert_eq!(pair("1.5e3"), "k: \"1.5e3\"");
    assert_eq!(pair("\"q\""), "k: '\"q\"'");
    assert_eq!(pair("both \" and '"), "k: both \" and '");
    assert_eq!(pair("\"both\" 'x'"), "k: \"\\\"both\\\" 'x'\"");
    assert_eq!(pair("tab\there"), "k: tab\there");
    assert_eq!(pair("bell\u{7}"), "k: \"bell\\a\"");
    assert_eq!(pair("a\nb\n"), "k: |\n  a\n  b");
    assert_eq!(pair("a\nb\n\n"), "k: |+\n  a\n  b\n");
    assert_eq!(pair("  indented\nnext"), "k: |2-\n    indented\n  next");
    assert_eq!(pair("ends\nin space \n  "), "k: \"ends\\nin space\\ \\n  \"");
    assert_eq!(pair("a line that is long enough to be folded\n second"), "k: |-\n  a line that is long enough to be folded\n   second");
    // a long double-quoted text gets real line breaks, as `yaml` writes it
    let long = "a line that is long enough to be folded\n second\n ";
    assert_eq!(pair(long), "k: \"a line that is long enough to be folded\n\n  \\ second\n\n  \\ \"");
    assert_eq!(stringify::stringify_pair("k", &YamlValue::List(vec![]), ""), "k: []");
    assert_eq!(stringify::stringify_pair("true", &YamlValue::Bool(true), ""), "\"true\": true");
    assert_eq!(stringify::stringify_pair("k", &YamlValue::List(vec!["a\nb".into()]), ""), "k:\n  - |-\n    a\n    b");
    assert_eq!(stringify::stringify_pair("k", &YamlValue::Num(-0.0), ""), "k: -0");
    assert_eq!(stringify::stringify_pair("k", &YamlValue::Num(1e21), ""), "k: 1e+21");
}

#[test]
fn every_written_text_reads_back() {
    let real = real();
    for value in [
        "a\nb\n",
        "a\nb\n\n",
        "  indented\nnext",
        "ends\nin space \n  ",
        "a line that is long enough to be folded\n second",
        "a line that is long enough to be folded\n second\n ",
        "\nstarts with a break",
        "bell\u{7}",
        "--- not a fence",
        "...",
        ":colon",
        "tab\there",
        "quote ' and \" both",
        "#hash",
        "-",
        "?",
        "null",
        "~",
        "0o17",
        ".inf",
    ] {
        let next = set(&real, "description", value);
        assert_eq!(field_text(&parse_doc(&next), "description", false), value, "{value:?} as {next:?}");
        assert!(split_doc(&next).frontmatter.is_some());
    }
    for number in [0.5, -3.0, 1e21, 1.5e-7, 123_456_789.0] {
        let next = set(&real, "maxTurns", number);
        assert_eq!(schema::js_number_parse(&parse_doc(&next).values["maxTurns"].source), number);
    }
}
