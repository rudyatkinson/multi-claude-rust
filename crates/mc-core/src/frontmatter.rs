//! YAML frontmatter: split, parse, field-level round-trip edits (`src/shared/frontmatter.ts`).
//!
//! The text of a Markdown file that opens with a YAML frontmatter between two `---` lines, as
//! Claude Code's skills, subagents, rules and memories are written.
//!
//! In an editor the whole text is the one thing that is true. A form is this text read
//! ([`parse_doc`]), and a change in the form is a change of the text ([`set_field`], [`set_body`])
//! that touches only the lines of that one field: comments, the order of the fields and fields the
//! form does not know stay exactly as they were written.
//!
//! The text here always has plain line feeds and no byte order mark (see [`crate::text_file`]).
//! Offsets ([`Split::start`], [`Entry::range`], ...) are byte offsets into that text.
//!
//! The Electron version reads YAML with the `yaml` npm package; this port reads it with yaml-rust2
//! (see `read.rs` for what is checked by hand on top of it) and writes it with a port of `yaml`'s
//! string rules (`stringify.rs`), so that a form writes the same lines as before.

mod read;
mod schema;
mod stringify;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ops::Range;

/// Where a file's frontmatter and body are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// What stands between the two fences, with its last line break; `None` without a frontmatter.
    pub frontmatter: Option<&'a str>,
    /// Where the frontmatter begins in the text: right after the opening fence.
    pub start: usize,
    /// Where the body begins: right after the closing fence.
    pub body_start: usize,
    /// The file opens a frontmatter on its first line and never closes it.
    pub unclosed: bool,
}

/// A value of the frontmatter, as it was written and as it reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Value {
    /// The value as it stands in the file, without the key.
    pub source: String,
    /// Its text, when it is a plain piece of text.
    pub string: Option<String>,
    /// Its entries, when it is a list of plain values.
    pub items: Option<Vec<String>>,
}

/// Why the fields cannot be read. `line` counts from 1, in the frontmatter for [`read_frontmatter`]
/// and in the whole file for [`parse_doc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FmError {
    pub message: String,
    pub line: Option<usize>,
}

/// One top-level field of a frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    /// The lines of the field within the frontmatter: from the start of its key's line up to the
    /// next field, without the empty lines and comment lines that go with the next one.
    pub range: Range<usize>,
    pub value: Value,
}

/// A frontmatter read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub entries: Vec<Entry>,
    /// Why the fields cannot be read, when they cannot. Claude Code then loads none of them either.
    pub error: Option<FmError>,
}

/// A whole file read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc<'a> {
    pub parts: Split<'a>,
    /// Why the fields cannot be read, when they cannot; the line counts lines of the file.
    pub error: Option<FmError>,
    /// The fields in the order they are written, each once.
    pub keys: Vec<String>,
    /// Of a field that is written twice the first one counts here; the file is flagged anyway.
    pub values: HashMap<String, Value>,
    /// The Markdown under the frontmatter, without the empty line that usually comes first.
    pub body: &'a str,
    /// Lines of the whole file.
    pub lines: usize,
}

/// What a field is set to. A text stands as typed where YAML reads it the same way and in quotes
/// otherwise; a number stands bare; several names become a YAML list. [`FieldInput::Remove`], an
/// empty text and an empty list take the field out.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldInput {
    Str(String),
    Bool(bool),
    Num(f64),
    List(Vec<String>),
    /// `null` in the TypeScript.
    Remove,
}

impl From<&str> for FieldInput {
    fn from(s: &str) -> Self {
        FieldInput::Str(s.to_owned())
    }
}
impl From<String> for FieldInput {
    fn from(s: String) -> Self {
        FieldInput::Str(s)
    }
}
impl From<bool> for FieldInput {
    fn from(b: bool) -> Self {
        FieldInput::Bool(b)
    }
}
impl From<f64> for FieldInput {
    fn from(n: f64) -> Self {
        FieldInput::Num(n)
    }
}
impl From<i64> for FieldInput {
    fn from(n: i64) -> Self {
        FieldInput::Num(n as f64)
    }
}
impl From<u32> for FieldInput {
    fn from(n: u32) -> Self {
        FieldInput::Num(f64::from(n))
    }
}
impl From<Vec<String>> for FieldInput {
    fn from(items: Vec<String>) -> Self {
        FieldInput::List(items)
    }
}
impl From<&[&str]> for FieldInput {
    fn from(items: &[&str]) -> Self {
        FieldInput::List(items.iter().map(|s| (*s).to_owned()).collect())
    }
}
impl<T: Into<FieldInput>> From<Option<T>> for FieldInput {
    fn from(value: Option<T>) -> Self {
        value.map_or(FieldInput::Remove, Into::into)
    }
}

/// Why [`set_field`] changed nothing; the text is a sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SetFieldFailure(pub String);

/// A value for [`frontmatter_block`]: what a JavaScript object literal handed to `stringify` holds.
/// Maps keep the order they are given in.
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<YamlValue>),
    Map(Vec<(String, YamlValue)>),
}

impl From<&str> for YamlValue {
    fn from(s: &str) -> Self {
        YamlValue::Str(s.to_owned())
    }
}
impl From<String> for YamlValue {
    fn from(s: String) -> Self {
        YamlValue::Str(s)
    }
}
impl From<bool> for YamlValue {
    fn from(b: bool) -> Self {
        YamlValue::Bool(b)
    }
}
impl From<f64> for YamlValue {
    fn from(n: f64) -> Self {
        YamlValue::Num(n)
    }
}
impl From<i64> for YamlValue {
    fn from(n: i64) -> Self {
        YamlValue::Num(n as f64)
    }
}
impl From<u32> for YamlValue {
    fn from(n: u32) -> Self {
        YamlValue::Num(f64::from(n))
    }
}
impl From<Vec<String>> for YamlValue {
    fn from(items: Vec<String>) -> Self {
        YamlValue::List(items.into_iter().map(YamlValue::Str).collect())
    }
}

// ---------- JavaScript string helpers ----------

/// JavaScript's white space for `trim()` and `\s`: Unicode white space but U+0085, and the byte
/// order mark.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `s.trim()` in JavaScript.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `s.trimEnd()` in JavaScript.
fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(is_js_space)
}

/// Where the line that holds `offset` begins.
fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

/// Moves an end back over empty lines and comments that start a line: they go with what follows.
fn without_trailing_notes(text: &str, from: usize, to: usize) -> usize {
    let mut end = to;
    while end > from {
        let start = line_start(text, end - 1);
        let line = js_trim_end(&text[start..end]);
        if !line.is_empty() && !line.starts_with('#') {
            break;
        }
        // the field's own first line is never given away
        if start <= from {
            break;
        }
        end = start;
    }
    end
}

// ---------- splitting ----------

fn fence_spaces(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t']).len()
}

/// Claude Code only sees a frontmatter that opens on the very first line: `^---[ \t]*\n`. It closes
/// at the first `\n---[ \t]*` that ends a line or the text; the line break before it may be the
/// opening fence's own, which makes an empty frontmatter.
pub fn split_doc(text: &str) -> Split<'_> {
    let none = |unclosed| Split { frontmatter: None, start: 0, body_start: 0, unclosed };
    let opening = text.strip_prefix("---").and_then(|rest| {
        let ws = fence_spaces(rest);
        rest[ws..].starts_with('\n').then_some(3 + ws + 1)
    });
    let Some(start) = opening else {
        let lone_fence = text.strip_prefix("---").is_some_and(|rest| rest.chars().all(|c| c == ' ' || c == '\t'));
        return none(lone_fence);
    };
    let search = &text[start - 1..];
    let mut from = 0;
    let closing = loop {
        let Some(p) = search[from..].find("\n---") else { break None };
        let at = from + p;
        let after = &search[at + 4..];
        let ws = fence_spaces(after);
        if after[ws..].is_empty() || after[ws..].starts_with('\n') {
            break Some((at, 4 + ws));
        }
        from = at + 1;
    };
    let Some((index, length)) = closing else { return none(true) };
    let end = start - 1 + index + 1;
    let after_fence = start - 1 + index + length;
    Split { frontmatter: Some(&text[start..end]), start, body_start: text.len().min(after_fence + 1), unclosed: false }
}

// ---------- reading ----------

/// Reads the text between the fences: its top-level fields, and why they cannot be read when they
/// cannot. A map in braces, or anything but a map, is an error too. After a YAML error the fields
/// read up to it are still given.
pub fn read_frontmatter(frontmatter: &str) -> Parsed {
    read::read_frontmatter(frontmatter)
}

/// The body as the editor shows it, and the empty line between the fence and it.
fn body_of<'a>(text: &'a str, parts: &Split<'_>) -> (&'static str, &'a str) {
    let rest = &text[parts.body_start..];
    if parts.frontmatter.is_none() {
        return ("", rest);
    }
    match rest.strip_prefix('\n') {
        Some(body) => ("\n", body),
        None => ("", rest),
    }
}

/// Reads a whole file. An error's line counts lines of the file.
pub fn parse_doc(text: &str) -> Doc<'_> {
    let parts = split_doc(text);
    let read = parts.frontmatter.map_or(Parsed { entries: Vec::new(), error: None }, read_frontmatter);
    let error = if parts.unclosed {
        Some(FmError { message: "The frontmatter that opens on the first line is never closed with a line of \"---\".".to_owned(), line: Some(1) })
    } else {
        // the frontmatter's first line is the file's second
        read.error.map(|e| FmError { line: e.line.map(|l| l + 1), ..e })
    };
    let mut keys: Vec<String> = Vec::new();
    let mut values: HashMap<String, Value> = HashMap::new();
    for entry in read.entries {
        if let std::collections::hash_map::Entry::Vacant(slot) = values.entry(entry.key) {
            keys.push(slot.key().clone());
            slot.insert(entry.value);
        }
    }
    let lines = if text.is_empty() { 0 } else { text.strip_suffix('\n').unwrap_or(text).split('\n').count() };
    Doc { parts, error, keys, values, body: body_of(text, &parts).1, lines }
}

/// `yes`, `on` and `1` are booleans to Claude Code as much as `true` is. `None` for anything else.
pub fn as_bool(text: &str) -> Option<bool> {
    match js_trim(text).to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// How a form shows a field: its text, the entries of a list joined by commas (with `list`), or
/// what is written.
pub fn field_text(doc: &Doc<'_>, key: &str, list: bool) -> String {
    let Some(value) = doc.values.get(key) else { return String::new() };
    if list {
        if let Some(items) = &value.items {
            return items.join(", ");
        }
    }
    value.string.clone().unwrap_or_else(|| value.source.clone())
}

/// A switch of the frontmatter; `fallback` when it is not set or not a yes or no.
pub fn field_bool(doc: &Doc<'_>, key: &str, fallback: bool) -> bool {
    doc.values.get(key).and_then(|v| as_bool(v.string.as_deref().unwrap_or(&v.source))).unwrap_or(fallback)
}

/// The entries of a field that holds several names: a YAML list, or one text with commas.
pub fn field_items(doc: &Doc<'_>, key: &str) -> Vec<String> {
    let Some(value) = doc.values.get(key) else { return Vec::new() };
    if let Some(items) = &value.items {
        return items.clone();
    }
    split_items(value.string.as_deref().unwrap_or(&value.source))
}

/// Names typed with commas between them.
pub fn split_items(text: &str) -> Vec<String> {
    text.split(',').map(js_trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// The name the file gives itself, when it does.
pub fn name_of(doc: &Doc<'_>) -> Option<String> {
    let name = field_text(doc, "name", false);
    let name = js_trim(&name);
    (!name.is_empty()).then(|| name.to_owned())
}

/// The first line of the text that says something, without the marks of a heading.
pub fn first_body_line(body: &str) -> String {
    let line = body.split('\n').find(|l| !js_trim(l).is_empty()).unwrap_or("");
    // `/^#+\s*/`, then trim
    let unmarked = if line.starts_with('#') { line.trim_start_matches('#') } else { line };
    js_trim(unmarked).to_owned()
}

// ---------- writing ----------

const FIX_FIRST: &str = "The frontmatter has a problem. Fix it in the Source tab first.";

/// `^[A-Za-z_][A-Za-z0-9_-]*$`
fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A way to write the field, tried in turn and read back.
struct Candidate {
    line: String,
    /// Only tried when yaml-rust2 cannot parse the candidate before it at all: there the `yaml`
    /// package would have read that one, so the TypeScript would have succeeded.
    fallback: bool,
}

/// The lines that could stand for the field, best first.
fn candidates(key: &str, input: &FieldInput, remove: bool) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut push = |line: String, fallback: bool| {
        if !out.iter().any(|c| c.line == line) {
            out.push(Candidate { line, fallback });
        }
    };
    if remove {
        push(String::new(), false);
        return out;
    }
    match input {
        FieldInput::Bool(b) => push(format!("{key}: {b}\n"), false),
        FieldInput::Num(n) => push(format!("{key}: {}\n", schema::js_number_string(*n)), false),
        FieldInput::Str(s) => {
            // as it was typed when YAML reads that the same way, else in quotes or as a block
            if !s.contains('\n') && s == js_trim(s) {
                push(format!("{key}: {s}\n"), false);
            }
            push(format!("{}\n", stringify::stringify_pair(key, &YamlValue::Str(s.clone()), "")), false);
            // a JSON string is valid YAML wherever it stands
            push(format!("{}: {}\n", stringify::stringify_string(key, "  ", true), stringify::json_string(s)), true);
        }
        FieldInput::List(items) => {
            let list = YamlValue::List(items.iter().cloned().map(YamlValue::Str).collect());
            push(format!("{}\n", stringify::stringify_pair(key, &list, "")), false);
            let quoted: String = items.iter().map(|item| format!("  - {}\n", stringify::json_string(item))).collect();
            push(format!("{}:\n{quoted}", stringify::stringify_string(key, "  ", true)), true);
        }
        FieldInput::Remove => {}
    }
    out
}

/// The value reads back as what was asked.
fn reads_as(value: &Value, input: &FieldInput) -> bool {
    match input {
        FieldInput::Bool(b) => as_bool(value.string.as_deref().unwrap_or(&value.source)) == Some(*b),
        FieldInput::Num(n) => value.string.is_none() && schema::js_number_parse(&value.source) == *n,
        // a text must read as that text; only a list in brackets may stand as it was typed
        FieldInput::Str(s) => value.string.as_deref() == Some(s.as_str()) || (value.string.is_none() && value.items.is_some() && value.source == *s),
        FieldInput::List(items) => value.items.as_ref() == Some(items),
        FieldInput::Remove => true,
    }
}

/// The text with one field set or taken out. Only the lines of that field change. The result is
/// read again before it is given back: when the frontmatter would not say exactly what was asked,
/// with every other field as it was, nothing is changed and the failure says so.
pub fn set_field(text: &str, key: &str, input: FieldInput) -> Result<String, SetFieldFailure> {
    if !is_key(key) {
        return Err(SetFieldFailure(format!("\"{key}\" cannot be set from the form.")));
    }
    if matches!(input, FieldInput::Num(n) if !n.is_finite()) {
        return Err(SetFieldFailure(format!("\"{key}\" needs a number.")));
    }
    let remove = match &input {
        FieldInput::Remove => true,
        FieldInput::Str(s) => s.is_empty(),
        FieldInput::List(items) => items.is_empty(),
        FieldInput::Bool(_) | FieldInput::Num(_) => false,
    };
    let parts = split_doc(text);
    if parts.unclosed {
        return Err(SetFieldFailure(FIX_FIRST.to_owned()));
    }
    let Some(frontmatter) = parts.frontmatter else {
        if remove {
            return Ok(text.to_owned());
        }
        // a file without a frontmatter gets one
        let gap = if text.is_empty() || text.starts_with('\n') { "" } else { "\n" };
        return set_field(&format!("---\n---\n{gap}{text}"), key, input);
    };
    let before = read_frontmatter(frontmatter);
    if before.error.is_some() {
        return Err(SetFieldFailure(FIX_FIRST.to_owned()));
    }
    let existing = before.entries.iter().find(|e| e.key == key);
    if remove && existing.is_none() {
        return Ok(text.to_owned());
    }

    let range = existing.map_or(frontmatter.len()..frontmatter.len(), |e| e.range.clone());
    // a last line without a line break would run into what is added after it
    let lead = if existing.is_none() && !frontmatter.is_empty() && !frontmatter.ends_with('\n') { "\n" } else { "" };
    let others: Vec<&Entry> = before.entries.iter().filter(|e| e.key != key).collect();
    let mut unparsed = false;
    for candidate in candidates(key, &input, remove) {
        if candidate.fallback && !unparsed {
            continue;
        }
        let next = format!("{}{lead}{}{}", &frontmatter[..range.start], candidate.line, &frontmatter[range.end..]);
        let after = read_frontmatter(&next);
        unparsed = after.error.is_some();
        if unparsed {
            continue;
        }
        let rest: Vec<&Entry> = after.entries.iter().filter(|e| e.key != key).collect();
        let untouched = rest.len() == others.len()
            && rest.iter().zip(&others).all(|(e, o)| e.key == o.key && next[e.range.clone()] == frontmatter[o.range.clone()]);
        if !untouched {
            continue;
        }
        let written: Vec<&Entry> = after.entries.iter().filter(|e| e.key == key).collect();
        let says = if remove { written.is_empty() } else { written.len() == 1 && reads_as(&written[0].value, &input) };
        if !says {
            continue;
        }
        return Ok(format!("{}{next}{}", &text[..parts.start], &text[parts.start + frontmatter.len()..]));
    }
    Err(SetFieldFailure(format!("This value cannot be written into the frontmatter from the form. Change \"{key}\" in the Source tab.")))
}

/// The text with another body under the same frontmatter.
pub fn set_body(text: &str, body: &str) -> String {
    let parts = split_doc(text);
    if parts.frontmatter.is_none() {
        return body.to_owned();
    }
    let (current_gap, current_body) = body_of(text, &parts);
    // an empty line after the fence, as files have by habit, when a body appears where none was
    let gap = if !current_gap.is_empty() {
        current_gap
    } else if current_body.is_empty() && !body.is_empty() {
        "\n"
    } else {
        ""
    };
    let head = &text[..parts.body_start];
    let head_break = if head.ends_with('\n') || body.is_empty() { "" } else { "\n" };
    format!("{head}{head_break}{}{body}", if body.is_empty() { current_gap } else { gap })
}

/// The frontmatter of a new file: the given fields, each written as YAML reads it back.
/// `---\n<yaml>---\n`; no fields make `{}`, as `stringify({})` does.
pub fn frontmatter_block(fields: &[(&str, YamlValue)]) -> String {
    format!("---\n{}---\n", stringify::stringify_map(fields))
}
