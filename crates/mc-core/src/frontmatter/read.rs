//! Reads a frontmatter with yaml-rust2's event parser and finds, for every top-level field, the
//! lines it stands on and what it says.
//!
//! The parser gives only the start of each node (as line and column; its byte index is a count of
//! characters, and not even that inside long block scalars), so the ends are found here on the
//! text: a plain scalar ends before a comment, a quoted one at its closing quote, a flow collection
//! at its closing bracket, a block collection where its last entry ends.
//!
//! Three checks `yaml` makes are made here because yaml-rust2 does not: a key written twice in one
//! map, a second document after a `...` line, and a line of white space with a tab in it that is
//! indented less than the block scalar it stands in.

use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::{Marker, ScanError, TScalarStyle};

use super::schema::{resolve_plain, resolve_tagged, Resolved};
use super::{js_trim, js_trim_end, line_start, without_trailing_notes, Entry, FmError, Parsed, Value};

/// `BLOCK_AS_IMPLICIT_KEY` in `yaml`; yaml-rust2 says "mapping values are not allowed in this context".
pub(super) const COLON_IN_VALUE: &str = "A value contains a colon followed by a space, and has to stand in quotes for that.";
/// `DUPLICATE_KEY` in `yaml`; yaml-rust2 does not check it, so the reader does.
pub(super) const WRITTEN_TWICE: &str = "A field is written twice.";
/// `TAB_AS_INDENT` in `yaml`; yaml-rust2 has several messages about tabs.
pub(super) const TAB_INDENT: &str = "A line is indented with a tab; YAML wants spaces.";
const NOT_A_MAP: &str = "The frontmatter is not a list of \"name: value\" lines.";
const IN_BRACES: &str = "The frontmatter is written in braces on one line, which the form cannot change.";
/// `MULTIPLE_DOCS` in `yaml`, said without its advice to call `parseAllDocuments()`.
const SEVERAL_DOCUMENTS: &str = "The frontmatter holds more than one YAML document.";
/// `BAD_INDENT` in `yaml` for a line of a block scalar, in `yaml`'s words.
const LESS_THAN_FIRST_LINE: &str = "Block scalar lines must not be less indented than their first line";
const LESS_THAN_INDICATOR: &str = "Block scalar lines must not be less indented than their explicit indentation indicator";

struct Node {
    /// Byte offset in the frontmatter.
    start: usize,
    /// 1-based line in the frontmatter.
    line: usize,
    kind: Kind,
}

enum Kind {
    Scalar {
        value: String,
        style: TScalarStyle,
        tag: Option<(String, String)>,
        /// Not written at all: an empty value, as after `key:`.
        empty: bool,
    },
    Alias,
    Seq {
        flow: bool,
        items: Vec<Node>,
    },
    Map {
        flow: bool,
        pairs: Vec<(Node, Node)>,
    },
    /// A value with a `: ` in it, up to `end`: `yaml` reads it as a nested map that takes in the
    /// lines after it, which is no text and no list entry.
    Broken {
        end: usize,
    },
}

impl Node {
    fn resolved(&self) -> Option<Resolved> {
        match &self.kind {
            Kind::Scalar { value, tag: Some((handle, suffix)), .. } => Some(resolve_tagged(value, handle, suffix)),
            Kind::Scalar { empty: true, .. } => Some(Resolved::Null),
            Kind::Scalar { value, style: TScalarStyle::Plain, .. } => Some(resolve_plain(value)),
            Kind::Scalar { value, .. } => Some(Resolved::Str(value.clone())),
            _ => None,
        }
    }
}

enum Frame {
    Seq { start: usize, line: usize, flow: bool, items: Vec<Node> },
    Map { start: Option<usize>, line: usize, flow: bool, pairs: Vec<(Node, Node)>, key: Option<Node>, seen: Vec<Resolved> },
}

impl Frame {
    fn flow(&self) -> bool {
        match self {
            Frame::Seq { flow, .. } | Frame::Map { flow, .. } => *flow,
        }
    }

    fn into_node(self, end_mark: usize) -> Node {
        match self {
            Frame::Seq { start, line, flow, items } => Node { start, line, kind: Kind::Seq { flow, items } },
            Frame::Map { start, line, flow, pairs, .. } => Node { start: start.unwrap_or(end_mark), line, kind: Kind::Map { flow, pairs } },
        }
    }
}

/// Byte offsets of the lines, to turn a marker's line and column (in characters) into a position.
/// `shifts` counts, per line, the tabs taken out of its indentation before it was parsed again.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
    shifts: &'a [usize],
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, shifts: &'a [usize]) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts, shifts }
    }

    fn at(&self, line: usize, col: usize) -> usize {
        let Some(&start) = line.checked_sub(1).and_then(|i| self.starts.get(i)) else {
            return self.text.len();
        };
        let col = col + self.shifts.get(line - 1).copied().unwrap_or(0);
        let end = self.text[start..].find('\n').map_or(self.text.len(), |i| start + i);
        self.text[start..end].char_indices().nth(col).map_or(end, |(i, _)| start + i)
    }

    fn offset(&self, mark: &Marker) -> usize {
        self.at(mark.line(), mark.col())
    }
}

/// A YAML error the reader can step over to read the fields after it, as `yaml` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// A `: ` inside a value, at this line and column of the text that was parsed.
    Colon { line: usize, col: usize },
    /// A tab in the indentation.
    Tab,
    Other,
}

struct Builder<'a> {
    fm: &'a str,
    lines: Lines<'a>,
    stack: Vec<Frame>,
    root: Option<Node>,
    error: Option<FmError>,
    failure: Option<Failure>,
}

impl Builder<'_> {
    fn fail(&mut self, message: &str, line: Option<usize>) {
        if self.error.is_none() {
            self.error = Some(FmError { message: message.to_owned(), line });
        }
    }

    fn add(&mut self, node: Node) {
        let mut twice = None;
        match self.stack.last_mut() {
            None => {
                if self.root.is_none() {
                    self.root = Some(node);
                }
            }
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map { start, pairs, key, seen, .. }) => match key.take() {
                Some(k) => pairs.push((k, node)),
                None => {
                    if start.is_none() {
                        *start = Some(node.start);
                    }
                    // `yaml` compares scalar keys by their value (`a.value === b.value`)
                    if let Some(id) = node.resolved() {
                        if seen.contains(&id) {
                            twice = Some(node.line);
                        } else {
                            seen.push(id);
                        }
                    }
                    *key = Some(node);
                }
            },
        }
        if let Some(line) = twice {
            self.fail(WRITTEN_TWICE, Some(line));
        }
    }

    fn in_flow(&self) -> bool {
        self.stack.last().is_some_and(Frame::flow)
    }

    fn scan_error(&mut self, err: &ScanError) {
        let info = err.info();
        let line = err.marker().line();
        if info.contains("mapping values are not allowed") {
            self.fail(COLON_IN_VALUE, Some(line));
            self.failure = Some(Failure::Colon { line, col: err.marker().col() });
        } else if info.contains("unknown anchor") {
            let message = match alias_name(self.fm, self.lines.offset(err.marker())) {
                Some(name) => format!("An alias (*{name}) points to an anchor (&{name}) that is not defined."),
                None => "An alias points to an anchor that is not defined.".to_owned(),
            };
            self.fail(&message, Some(line));
            self.failure = Some(Failure::Other);
        } else if info.split(|c: char| !c.is_ascii_alphabetic()).any(|w| w == "tab" || w == "tabs") {
            // yaml-rust2 may name the line before the one with the tab
            self.fail(TAB_INDENT, Some(first_tab_indented_line(self.fm, line).unwrap_or(line)));
            self.failure = Some(Failure::Tab);
        } else {
            let mut chars = info.chars();
            let message = chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default();
            self.fail(&message, Some(line));
            self.failure = Some(Failure::Other);
        }
    }
}

/// The name of the alias (`*name`) yaml-rust2 marks at `at`.
fn alias_name(fm: &str, at: usize) -> Option<&str> {
    let line_end = fm[at.min(fm.len())..].find('\n').map_or(fm.len(), |i| at + i);
    let star = at + fm[at..line_end].find('*')?;
    let name = &fm[star + 1..line_end];
    let len = name.find(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '{' | '}')).unwrap_or(name.len());
    (len > 0).then(|| &name[..len])
}

/// `MULTIPLE_DOCS` in `yaml` after a `...` line, which ends the document (alone on its line, or
/// followed by white space or a comment): the first line after it that is neither empty nor a
/// comment starts another one. yaml-rust2 does not see that one when the `...` is the first line.
/// Where the `...` line begins and the line (from 1) of the second document.
fn second_document(fm: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut ended = None;
    for (index, line) in fm.split('\n').enumerate() {
        match ended {
            None => {
                let ends = line.strip_prefix("...").is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '#']));
                if ends {
                    ended = Some(offset);
                }
            }
            Some(at) => {
                let content = js_trim(line);
                if !content.is_empty() && !content.starts_with('#') {
                    return Some((at, index + 1));
                }
            }
        }
        offset += line.len() + 1;
    }
    None
}

/// `BAD_INDENT` in `yaml` for the block scalar whose `|` or `>` stands at `header`: a line of
/// white space with a tab in it that has fewer spaces in front than the scalar's content (the
/// first line of it, or the indentation indicator). A line with text that is indented less ends
/// the scalar. The line (from 1) and what `yaml` says of it.
fn block_scalar_bad_indent(fm: &str, header: usize) -> Option<(usize, &'static str)> {
    let spaces = |s: &str| s.len() - s.trim_start_matches(' ').len();
    // the indentation of the map or list the scalar is a value of
    let parent = spaces(&fm[line_start(fm, header)..]);
    let header_end = header + fm[header..].find('\n')?;
    let indicator = fm[header + 1..header_end]
        .bytes()
        .take_while(|b| !matches!(b, b' ' | b'\t' | b'#'))
        .find(u8::is_ascii_digit)
        .map(|d| usize::from(d - b'0'));
    let message = if indicator.is_some() { LESS_THAN_INDICATOR } else { LESS_THAN_FIRST_LINE };
    let mut content = indicator.map(|n| parent + n);
    let mut number = fm[..header].matches('\n').count() + 1;
    for line in fm[header_end + 1..].split('\n') {
        number += 1;
        let indent = spaces(line);
        let rest = &line[indent..];
        // only spaces: an empty line of the scalar
        if rest.is_empty() {
            continue;
        }
        let blank = rest.chars().all(|c| c == ' ' || c == '\t');
        match content {
            None if indent <= parent && !blank => return None,
            None => content = Some(indent),
            Some(want) if indent >= want => {}
            Some(_) if blank => return Some((number, message)),
            Some(_) => return None,
        }
    }
    None
}

/// The first line, from `from` on, whose indentation holds a tab.
fn first_tab_indented_line(text: &str, from: usize) -> Option<usize> {
    text.split('\n').enumerate().skip(from.saturating_sub(1)).find_map(|(i, line)| {
        let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        indent.contains('\t').then_some(i + 1)
    })
}

/// Where the `|` or `>` of a block scalar stands. yaml-rust2 marks where its content begins (or,
/// when it has none, the next token), so the header is looked for backwards from there.
fn block_header_start(fm: &str, from: usize) -> Option<usize> {
    let bytes = fm.as_bytes();
    let mut p = from.min(fm.len());
    loop {
        if p < fm.len() && matches!(bytes[p], b'|' | b'>') && (p == 0 || matches!(bytes[p - 1], b' ' | b'\t' | b'\n')) {
            let line_end = fm[p..].find('\n').map_or(fm.len(), |i| p + i);
            let rest = fm[p + 1..line_end].trim_start_matches(|c: char| c.is_ascii_digit() || c == '+' || c == '-');
            let after = rest.trim_start_matches([' ', '\t']);
            if after.is_empty() || (after.starts_with('#') && after.len() < rest.len()) {
                return Some(p);
            }
        }
        if p == 0 {
            return None;
        }
        p -= 1;
    }
}

/// The `-` of a block sequence. yaml-rust2 marks it for an indented list but marks the first
/// entry's content for a list at the key's own indentation.
fn dash_start(fm: &str, at: usize) -> usize {
    let bytes = fm.as_bytes();
    let mut p = at.min(fm.len());
    if p < fm.len() && bytes[p] == b'-' {
        return p;
    }
    while p > 0 && matches!(bytes[p - 1], b' ' | b'\t') {
        p -= 1;
    }
    if p > 0 && bytes[p - 1] == b'-' {
        p - 1
    } else {
        at
    }
}

struct Tree {
    root: Option<Node>,
    error: Option<FmError>,
    failure: Option<Failure>,
}

/// Parses `parse`, a copy of the frontmatter `fm` that may have been mended (see
/// [`read_frontmatter`]); every position is one in `fm`.
fn build(fm: &str, parse: &str, shifts: &[usize]) -> Tree {
    let mut b = Builder { fm, lines: Lines::new(fm, shifts), stack: Vec::new(), root: None, error: None, failure: None };
    let mut parser = Parser::new_from_str(parse);
    let mut documents = 0;
    loop {
        let (event, mark) = match parser.next_token() {
            Ok(next) => next,
            Err(err) => {
                b.scan_error(&err);
                break;
            }
        };
        let at = b.lines.offset(&mark);
        let line = mark.line();
        match event {
            Event::StreamEnd => break,
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 {
                    b.fail(SEVERAL_DOCUMENTS, Some(line));
                    break;
                }
            }
            Event::Alias(_) => b.add(Node { start: at, line, kind: Kind::Alias }),
            Event::Scalar(value, style, _, tag) => {
                let empty = style == TScalarStyle::Plain && value.is_empty();
                let start = match style {
                    TScalarStyle::Literal | TScalarStyle::Folded => {
                        let header = block_header_start(fm, at);
                        // `yaml` reads on after it, so the fields still count
                        if let Some((line, message)) = header.and_then(|h| block_scalar_bad_indent(fm, h)) {
                            b.fail(message, Some(line));
                        }
                        header.unwrap_or(at)
                    }
                    _ => at,
                };
                let tag = tag.map(|t| (t.handle, t.suffix));
                b.add(Node { start, line, kind: Kind::Scalar { value, style, tag, empty } });
            }
            Event::SequenceStart(..) => {
                let flow = b.in_flow() || fm[at..].starts_with('[');
                let start = if flow { at } else { dash_start(fm, at) };
                b.stack.push(Frame::Seq { start, line, flow, items: Vec::new() });
            }
            Event::MappingStart(..) => {
                let flow = b.in_flow() || fm[at..].starts_with('{');
                // a block map begins with its first key; yaml-rust2 marks its first `:`
                b.stack.push(Frame::Map { start: flow.then_some(at), line, flow, pairs: Vec::new(), key: None, seen: Vec::new() });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some(frame) = b.stack.pop() {
                    let node = frame.into_node(at);
                    b.add(node);
                }
            }
            Event::Nothing | Event::StreamStart | Event::DocumentEnd => {}
        }
    }
    // after an error, what was read so far still counts
    while let Some(frame) = b.stack.pop() {
        let node = frame.into_node(fm.len());
        b.add(node);
    }
    Tree { root: b.root, error: b.error, failure: b.failure }
}

/// How many errors the reader steps over before it gives what it has.
const MENDS: usize = 16;

/// The scalar that starts last before `at`, and whether it is a plain one.
fn last_scalar_before(node: &Node, at: usize, best: &mut Option<(usize, bool)>) {
    match &node.kind {
        Kind::Scalar { style, empty: false, .. } if node.start < at => {
            if best.is_none_or(|(s, _)| node.start > s) {
                *best = Some((node.start, *style == TScalarStyle::Plain));
            }
        }
        Kind::Seq { items, .. } => items.iter().for_each(|item| last_scalar_before(item, at, best)),
        Kind::Map { pairs, .. } => pairs.iter().for_each(|(k, v)| {
            last_scalar_before(k, at, best);
            last_scalar_before(v, at, best);
        }),
        _ => {}
    }
}

/// Where the nested map `yaml` makes of a value with a `: ` in it ends: it takes in the lines
/// after it that are indented deeper than the line it starts on, or as deep and not a `- ` entry.
fn absorbed_end(fm: &str, start: usize) -> usize {
    let begin = line_start(fm, start);
    let indent = fm[begin..].len() - fm[begin..].trim_start_matches(' ').len();
    let mut p = fm[start..].find('\n').map_or(fm.len(), |i| start + i + 1);
    let mut end = p;
    while p < fm.len() {
        let next = fm[p..].find('\n').map_or(fm.len(), |i| p + i + 1);
        let line = &fm[p..next];
        let content = line.trim_start_matches(' ');
        let depth = line.len() - content.len();
        if js_trim(line).is_empty() || content.starts_with('#') {
            p = next;
            continue;
        }
        let entry = content.starts_with("- ") || js_trim(content) == "-";
        if depth > indent || (depth == indent && !entry) {
            p = next;
            end = next;
            continue;
        }
        break;
    }
    end
}

/// An offset of the frontmatter as the offset of the same character in its mended copy, whose
/// lines lost `shifts` tabs of their indentation.
fn mended_offset(fm: &str, text: &str, shifts: &[usize], at: usize) -> usize {
    let line = fm[..at].matches('\n').count();
    let col = fm[line_start(fm, at)..at].chars().count().saturating_sub(shifts.get(line).copied().unwrap_or(0));
    Lines::new(text, &[]).at(line + 1, col)
}

/// `yaml` reads on after an error; yaml-rust2 stops at it. For the two mistakes people make most
/// the frontmatter is mended and read again, so that the fields `yaml` still reads are known too:
///
/// - a value with a `: ` in it is blanked out with the lines `yaml` takes into it (see
///   [`absorbed_end`]); it then reads as [`Kind::Broken`], and the error's line is the line the
///   value starts on, as `yaml` names it;
/// - tabs in the indentation are taken out, which reads the fields as `yaml` does.
///
/// The error the user sees stays the first one of the text as written. Every mend keeps the
/// number of characters of each line but for the tabs, so positions map back to the frontmatter.
fn build_mended(fm: &str) -> (Tree, Vec<(usize, usize)>) {
    let first = build(fm, fm, &[]);
    let mut error = first.error.clone();
    let mut tree = first;
    let mut text = fm.to_owned();
    let mut shifts: Vec<usize> = Vec::new();
    let mut broken: Vec<(usize, usize)> = Vec::new();
    for round in 0..MENDS {
        match tree.failure {
            Some(Failure::Colon { line, col }) => {
                let at = Lines::new(fm, &shifts).at(line, col);
                let mut best = None;
                if let Some(root) = &tree.root {
                    last_scalar_before(root, at, &mut best);
                }
                let Some((start, true)) = best else { break };
                if round == 0 {
                    if let Some(e) = error.as_mut().filter(|e| e.message == COLON_IN_VALUE) {
                        e.line = Some(fm[..start].matches('\n').count() + 1);
                    }
                }
                let end = absorbed_end(fm, start);
                let (s, e) = (mended_offset(fm, &text, &shifts, start), mended_offset(fm, &text, &shifts, end));
                // a placeholder scalar where the value starts, spaces for the rest
                let blank: String = text[s..e].chars().enumerate().map(|(i, c)| if c == '\n' { '\n' } else if i == 0 { '_' } else { ' ' }).collect();
                text.replace_range(s..e, &blank);
                broken.push((start, end));
            }
            Some(Failure::Tab) if shifts.is_empty() => {
                let mut mended = String::with_capacity(text.len());
                for (i, line) in text.split('\n').enumerate() {
                    if i > 0 {
                        mended.push('\n');
                    }
                    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                    let tabs = line[..indent].matches('\t').count();
                    shifts.push(tabs);
                    mended.push_str(&line[..indent].replace('\t', ""));
                    mended.push_str(&line[indent..]);
                }
                text = mended;
            }
            _ => break,
        }
        tree = build(fm, &text, &shifts);
    }
    tree.error = error;
    (tree, broken)
}

// ---------- where a node ends ----------

fn plain_end(fm: &str, start: usize, limit: usize, in_flow: bool) -> usize {
    let s = &fm[start..limit];
    let mut end = s.len();
    let mut prev: Option<char> = None;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        let stops = (c == '#' && matches!(prev, Some(' ' | '\t' | '\n')))
            || (in_flow && matches!(c, ',' | '[' | ']' | '{' | '}'))
            || (in_flow && c == ':' && matches!(next, None | Some(' ' | '\t' | '\n' | ',' | ']' | '}')));
        if stops {
            end = i;
            break;
        }
        prev = Some(c);
    }
    start + js_trim_end(&s[..end]).len()
}

fn single_quote_end(fm: &str, start: usize, limit: usize) -> Option<usize> {
    let b = fm.as_bytes();
    let mut i = start + 1;
    while i < limit {
        if b[i] == b'\'' {
            if i + 1 < limit && b[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn double_quote_end(fm: &str, start: usize, limit: usize) -> Option<usize> {
    let b = fm.as_bytes();
    let mut i = start + 1;
    while i < limit {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn flow_end(fm: &str, start: usize, limit: usize) -> Option<usize> {
    let b = fm.as_bytes();
    let mut depth = 0usize;
    let mut prev = b' ';
    let mut i = start;
    while i < limit {
        let c = b[i];
        match c {
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b'#' if matches!(prev, b' ' | b'\t' | b'\n') => {
                i = fm[i..limit].find('\n').map_or(limit, |n| i + n);
                prev = b'\n';
                continue;
            }
            b'\'' | b'"' if matches!(prev, b' ' | b'\t' | b'\n' | b'[' | b'{' | b',' | b':') => {
                let end = if c == b'\'' { single_quote_end(fm, i, limit) } else { double_quote_end(fm, i, limit) };
                i = end?;
                prev = c;
                continue;
            }
            _ => {}
        }
        prev = c;
        i += 1;
    }
    None
}

/// Where a node's text ends when nothing better is known: before trailing comments.
fn fallback_end(fm: &str, start: usize, limit: usize) -> usize {
    let mut s = js_trim_end(&fm[start..limit]);
    loop {
        let ls = s.rfind('\n').map_or(0, |i| i + 1);
        if ls > 0 && s[ls..].trim_start().starts_with('#') {
            s = js_trim_end(&s[..ls]);
            continue;
        }
        break;
    }
    let ls = s.rfind('\n').map_or(0, |i| i + 1);
    let last = &s[ls..];
    let comment = last.char_indices().zip(last.chars().skip(1)).find(|&((_, c), n)| (c == ' ' || c == '\t') && n == '#').map(|((i, _), _)| i);
    if let Some(p) = comment {
        s = js_trim_end(&s[..ls + p]);
    }
    start + s.len()
}

fn node_end(fm: &str, node: &Node, limit: usize, in_flow: bool) -> Option<usize> {
    let start = node.start;
    if start >= limit {
        return None;
    }
    match &node.kind {
        Kind::Scalar { empty: true, .. } => None,
        Kind::Scalar { style: TScalarStyle::Plain, .. } => Some(plain_end(fm, start, limit, in_flow)),
        Kind::Scalar { style: TScalarStyle::SingleQuoted, .. } => single_quote_end(fm, start, limit),
        Kind::Scalar { style: TScalarStyle::DoubleQuoted, .. } => double_quote_end(fm, start, limit),
        Kind::Scalar { .. } => Some(start + js_trim_end(&fm[start..limit]).len()),
        Kind::Alias => {
            let s = &fm[start..limit];
            let len = s.find(|c: char| c.is_whitespace() || (in_flow && matches!(c, ',' | ']' | '}'))).unwrap_or(s.len());
            Some(start + len)
        }
        Kind::Broken { end } => Some(fallback_end(fm, start, (*end).clamp(start, limit))),
        Kind::Seq { flow: true, .. } | Kind::Map { flow: true, .. } => flow_end(fm, start, limit),
        Kind::Seq { flow: false, items } => items.last().and_then(|item| node_end(fm, item, limit, false)),
        Kind::Map { flow: false, pairs } => pairs.last().and_then(|(key, value)| {
            node_end(fm, value, limit, false).or_else(|| {
                // an empty last value: the map ends with its key and colon
                let end = node_end(fm, key, limit, false)?;
                let rest = &fm[end..limit];
                let colon = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                Some(if rest[colon..].starts_with(':') { end + colon + 1 } else { end })
            })
        }),
    }
}

/// The value as it stands in the file, without the key.
fn source_of(fm: &str, node: &Node, limit: usize, in_flow: bool) -> String {
    if matches!(node.kind, Kind::Scalar { empty: true, .. }) || node.start >= limit {
        return String::new();
    }
    let end = node_end(fm, node, limit, in_flow).unwrap_or_else(|| fallback_end(fm, node.start, limit));
    js_trim(&fm[node.start..end.max(node.start)]).to_owned()
}

fn value_of(fm: &str, node: &Node, limit: usize, in_flow: bool) -> Value {
    let mut value = Value { source: source_of(fm, node, limit, in_flow), string: None, items: None };
    match &node.kind {
        Kind::Scalar { .. } => {
            if let Some(Resolved::Str(s)) = node.resolved() {
                value.string = Some(s);
            }
        }
        Kind::Seq { items, .. } => {
            // a list of plain values; a null, an alias or a collection among them is not
            value.items = items
                .iter()
                .map(|item| match item.resolved() {
                    None | Some(Resolved::Null) => None,
                    Some(r) => Some(r.js_string()),
                })
                .collect();
        }
        _ => {}
    }
    value
}

/// Turns the placeholder scalar at `start` of a mended frontmatter into the broken value it stands for.
fn mark_broken(node: &mut Node, start: usize, end: usize) {
    match &mut node.kind {
        Kind::Scalar { style: TScalarStyle::Plain, .. } if node.start == start => node.kind = Kind::Broken { end },
        Kind::Seq { items, .. } => items.iter_mut().for_each(|item| mark_broken(item, start, end)),
        Kind::Map { pairs, .. } => pairs.iter_mut().for_each(|(_, v)| mark_broken(v, start, end)),
        _ => {}
    }
}

/// A value of the top-level map ends at the first line that starts in the first column with
/// something other than a comment, a `-` entry or the `:` of an explicit `? key`: the next field,
/// or a `...` line.
fn root_limit(fm: &str, from: usize, to: usize) -> usize {
    let mut p = fm[from..to].find('\n').map_or(to, |i| from + i + 1);
    while p < to {
        if !matches!(fm.as_bytes()[p], b' ' | b'\t' | b'#' | b'-' | b':' | b'\n') {
            return p;
        }
        p = fm[p..to].find('\n').map_or(to, |i| p + i + 1);
    }
    to
}

pub(super) fn read_frontmatter(fm: &str) -> Parsed {
    // `yaml` reads the first document and says that there is another; the fields end before it
    let second = second_document(fm);
    let (tree, broken) = build_mended(second.map_or(fm, |(at, _)| &fm[..at]));
    let mut error = tree.error;
    if let Some((_, line)) = second.filter(|_| error.is_none()) {
        error = Some(FmError { message: SEVERAL_DOCUMENTS.to_owned(), line: Some(line) });
    }
    let Some(mut root) = tree.root else { return Parsed { entries: Vec::new(), error } };
    for (start, end) in broken {
        mark_broken(&mut root, start, end);
    }
    let Kind::Map { flow, pairs } = &root.kind else {
        if error.is_none() {
            error = Some(FmError { message: NOT_A_MAP.to_owned(), line: None });
        }
        return Parsed { entries: Vec::new(), error };
    };
    if error.is_none() && *flow {
        error = Some(FmError { message: IN_BRACES.to_owned(), line: None });
    }

    let starts: Vec<usize> = pairs.iter().map(|(key, _)| line_start(fm, key.start.min(fm.len()))).collect();
    let mut entries = Vec::with_capacity(pairs.len());
    for (index, (key, value)) in pairs.iter().enumerate() {
        let from = starts[index];
        let next = starts[index + 1..].iter().copied().find(|&s| s > from).unwrap_or(fm.len());
        let to = without_trailing_notes(fm, from, next);
        let name = match key.resolved() {
            Some(r) => r.js_string(),
            None => source_of(fm, key, to, *flow),
        };
        let limit = match value.kind {
            _ if *flow => fm.len(),
            Kind::Broken { .. } => to,
            _ => root_limit(fm, from, to),
        };
        entries.push(Entry { key: name, range: from..to, value: value_of(fm, value, limit, *flow) });
    }
    Parsed { entries, error }
}
