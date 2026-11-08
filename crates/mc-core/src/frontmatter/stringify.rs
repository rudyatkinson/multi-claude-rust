//! Writes YAML the way `stringify(value, { lineWidth: 0 })` of the `yaml` npm package (2.x) does
//! for what a frontmatter holds: block maps and lists, and strings that stay plain where YAML reads
//! them back as the same text, else in double quotes (single quotes when the text holds a `"` but
//! no `'`), and several lines as a literal block (`|`, `|-`, `|+`).
//!
//! yaml-rust2 has an emitter, but it quotes differently; the frontmatter a form writes has to look
//! as the Electron version wrote it, so the rules of `yaml`'s `stringifyString` are ported here.
//! Folding is off (`lineWidth: 0`), so no line is ever broken.

use std::sync::LazyLock;

use regex::Regex;

use super::schema::{js_number_string, reads_as_non_string};
use super::YamlValue;
use crate::jsstr::len16;

const INDENT_STEP: &str = "  ";
/// `doubleQuotedMinMultiLineLength`: a shorter double-quoted text keeps its `\n` escapes.
const DOUBLE_QUOTED_MIN_MULTI_LINE: usize = 40;
/// A longer implicit key is written as an explicit `? key`.
const MAX_IMPLICIT_KEY: usize = 1024;

static PLAIN_UNSAFE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^[\n\t ,\[\]{}#&*!|>'"%@`]|^[?-]$|^[?-][ \t]|[\n:][ \t]|[ \t]\n|[\n\t ]#|[\n\t :]$"#).expect("a valid pattern")
});
static SPACE_AROUND_NEWLINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]\n|\n[ \t]").expect("a valid pattern"));
static BLOCK_ENDS_IN_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n[\t ]+$").expect("a valid pattern"));

/// `/^(%|---|\.\.\.)/m`: a line that a YAML 1.1 reader would take for the start of a document.
fn contains_document_marker(s: &str) -> bool {
    s.split(['\n', '\r', '\u{2028}', '\u{2029}']).any(|line| line.starts_with('%') || line.starts_with("---") || line.starts_with("..."))
}

/// Puts `after` behind every run of line feeds; with `skip_at_end`, not behind a run that ends the
/// text (`/\n+(?!\n|$)/g` instead of `/\n+/g`).
fn after_newline_runs(s: &str, after: &str, skip_at_end: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '\n' && chars.peek() != Some(&'\n') && !(skip_at_end && chars.peek().is_none()) {
            out.push_str(after);
        }
    }
    out
}

/// `JSON.stringify` of a string. Its escapes are all valid in a YAML double-quoted scalar too.
pub(super) fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn double_quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    let json = json_string(value);
    let indent = if !indent.is_empty() {
        indent
    } else if contains_document_marker(value) {
        INDENT_STEP
    } else {
        ""
    };
    let j: Vec<char> = json.chars().collect();
    let at = |k: usize| j.get(k).copied();
    let short = len16(&json) < DOUBLE_QUOTED_MIN_MULTI_LINE;
    let mut out = String::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < j.len() {
        let mut ch = j[i];
        if ch == ' ' && at(i + 1) == Some('\\') && at(i + 2) == Some('n') {
            // a space before a line break is escaped, or folding would eat it
            out.extend(&j[start..i]);
            out.push_str("\\ ");
            i += 1;
            start = i;
            ch = '\\';
        }
        if ch == '\\' {
            match at(i + 1) {
                Some('u') => {
                    out.extend(&j[start..i]);
                    let code: String = j[(i + 2).min(j.len())..(i + 6).min(j.len())].iter().collect();
                    match code.as_str() {
                        "0000" => out.push_str("\\0"),
                        "0007" => out.push_str("\\a"),
                        "000b" => out.push_str("\\v"),
                        "001b" => out.push_str("\\e"),
                        "0085" => out.push_str("\\N"),
                        "00a0" => out.push_str("\\_"),
                        "2028" => out.push_str("\\L"),
                        "2029" => out.push_str("\\P"),
                        c if c.starts_with("00") => {
                            out.push_str("\\x");
                            out.push_str(&c[2..]);
                        }
                        _ => out.extend(&j[i..(i + 6).min(j.len())]),
                    }
                    i += 5;
                    start = i + 1;
                }
                Some('n') => {
                    if implicit_key || at(i + 2) == Some('"') || short {
                        i += 1;
                    } else {
                        // a real line break; folding eats the first one
                        out.extend(&j[start..i]);
                        out.push_str("\n\n");
                        while at(i + 2) == Some('\\') && at(i + 3) == Some('n') && at(i + 4) != Some('"') {
                            out.push('\n');
                            i += 2;
                        }
                        out.push_str(indent);
                        // a space after a line break is escaped, or folding would eat it
                        if at(i + 2) == Some(' ') {
                            out.push('\\');
                        }
                        i += 1;
                        start = i + 1;
                    }
                }
                _ => i += 1,
            }
        }
        i += 1;
    }
    if start == 0 {
        return json;
    }
    out.extend(&j[start.min(j.len())..]);
    out
}

fn single_quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    if (implicit_key && value.contains('\n')) || SPACE_AROUND_NEWLINE.is_match(value) {
        // single quotes cannot keep white space next to a line break
        return double_quoted(value, indent, implicit_key);
    }
    let indent = if !indent.is_empty() {
        indent
    } else if contains_document_marker(value) {
        INDENT_STEP
    } else {
        ""
    };
    let escaped = value.replace('\'', "''");
    format!("'{}'", after_newline_runs(&escaped, &format!("\n{indent}"), false))
}

fn quoted(value: &str, indent: &str, implicit_key: bool) -> String {
    if value.contains('"') && !value.contains('\'') {
        single_quoted(value, indent, implicit_key)
    } else {
        double_quoted(value, indent, implicit_key)
    }
}

/// A text of several lines as a literal block. `lineWidth: 0` never folds, so it is never `>`.
fn block_string(value: &str, indent: &str, force_block_indent: bool) -> String {
    if BLOCK_ENDS_IN_SPACE.is_match(value) {
        // a block cannot end in white space unless its last line holds something
        return quoted(value, indent, false);
    }
    let indent = if !indent.is_empty() {
        indent
    } else if force_block_indent || contains_document_marker(value) {
        INDENT_STEP
    } else {
        ""
    };
    if value.is_empty() {
        return "|\n".to_owned();
    }

    // chomping, from the white space at the end
    let end_start = value.trim_end_matches(['\n', '\t', ' ']).len();
    let end = &value[end_start..];
    let chomp = match end.find('\n') {
        None => "-",
        Some(nl) if value == end || nl != end.len() - 1 => "+",
        Some(_) => "",
    };
    let mut content = value;
    let mut end_text = String::new();
    if !end.is_empty() {
        content = &value[..end_start];
        let end = end.strip_suffix('\n').unwrap_or(end);
        end_text = after_newline_runs(end, indent, true);
    }

    // an indentation indicator, from the white space at the start
    let mut start_with_space = false;
    let mut start_nl: Option<usize> = None;
    for (idx, ch) in content.char_indices() {
        match ch {
            ' ' => start_with_space = true,
            '\n' => start_nl = Some(idx),
            _ => break,
        }
    }
    let start = &content[..start_nl.map_or(0, |nl| nl + 1)];
    let start_text = after_newline_runs(start, indent, false);
    let content = &content[start.len()..];

    let indent_size = if indent.is_empty() { "1" } else { "2" };
    let header = format!("{}{chomp}", if start_with_space { indent_size } else { "" });
    let body = after_newline_runs(content, indent, false);
    format!("|{header}\n{indent}{start_text}{body}{end_text}")
}

fn plain_string(value: &str, indent: &str, implicit_key: bool) -> String {
    if implicit_key && value.contains('\n') {
        return quoted(value, indent, implicit_key);
    }
    if PLAIN_UNSAFE.is_match(value) {
        return if implicit_key || !value.contains('\n') { quoted(value, indent, implicit_key) } else { block_string(value, indent, false) };
    }
    if !implicit_key && value.contains('\n') {
        return block_string(value, indent, false);
    }
    if contains_document_marker(value) {
        if indent.is_empty() {
            return block_string(value, indent, true);
        } else if implicit_key && indent == INDENT_STEP {
            return quoted(value, indent, implicit_key);
        }
    }
    // what reads as a number, a switch or nothing has to stand in quotes to stay a text
    if reads_as_non_string(value) {
        return quoted(value, indent, implicit_key);
    }
    value.to_owned()
}

/// `stringifyString` for a string node without a style of its own.
pub(super) fn stringify_string(value: &str, indent: &str, implicit_key: bool) -> String {
    let control = value.chars().any(|c| matches!(c, '\0'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}'));
    if control {
        return double_quoted(value, indent, implicit_key);
    }
    plain_string(value, indent, implicit_key)
}

fn stringify_number(n: f64) -> String {
    if n.is_nan() {
        ".nan".to_owned()
    } else if n.is_infinite() {
        if n < 0.0 {
            "-.inf".to_owned()
        } else {
            ".inf".to_owned()
        }
    } else if n == 0.0 && n.is_sign_negative() {
        "-0".to_owned()
    } else {
        js_number_string(n)
    }
}

/// A value whose own lines stand at `indent`, not as a key.
fn stringify_value(value: &YamlValue, indent: &str) -> String {
    match value {
        YamlValue::Null => "null".to_owned(),
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Num(n) => stringify_number(*n),
        YamlValue::Str(s) => stringify_string(s, indent, false),
        YamlValue::List(items) if items.is_empty() => "[]".to_owned(),
        YamlValue::List(items) => {
            let item_indent = format!("{indent}{INDENT_STEP}");
            let lines: Vec<String> = items.iter().map(|item| format!("- {}", stringify_value(item, &item_indent))).collect();
            lines.join(&format!("\n{indent}"))
        }
        YamlValue::Map(pairs) if pairs.is_empty() => "{}".to_owned(),
        YamlValue::Map(pairs) => {
            let lines: Vec<String> = pairs.iter().map(|(k, v)| stringify_pair(k, v, indent)).collect();
            lines.join(&format!("\n{indent}"))
        }
    }
}

/// One `key: value` of a block map whose keys stand at `indent`, without the last line break.
pub(super) fn stringify_pair(key: &str, value: &YamlValue, indent: &str) -> String {
    let inner = format!("{indent}{INDENT_STEP}");
    let key_text = stringify_string(key, &inner, true);
    let explicit = len16(&key_text) > MAX_IMPLICIT_KEY;
    let head = if explicit { format!("? {key_text}\n{indent}:") } else { format!("{key_text}:") };
    let value_text = stringify_value(value, &inner);
    let collection_has_items = match value {
        YamlValue::List(items) => Some(!items.is_empty()),
        YamlValue::Map(pairs) => Some(!pairs.is_empty()),
        _ => None,
    };
    let ws = match collection_has_items {
        // a collection with entries starts on the next line; an empty one is `[]` or `{}` after the key
        Some(has_items) if !explicit => {
            if value_text.contains('\n') || has_items {
                format!("\n{inner}")
            } else {
                " ".to_owned()
            }
        }
        _ if value_text.is_empty() || value_text.starts_with('\n') => String::new(),
        _ => " ".to_owned(),
    };
    format!("{head}{ws}{value_text}")
}

/// `stringify(map, { lineWidth: 0 })`: the map as a YAML document, with its last line break.
pub(super) fn stringify_map(pairs: &[(&str, YamlValue)]) -> String {
    if pairs.is_empty() {
        return "{}\n".to_owned();
    }
    let lines: Vec<String> = pairs.iter().map(|(k, v)| stringify_pair(k, v, "")).collect();
    format!("{}\n", lines.join("\n"))
}
