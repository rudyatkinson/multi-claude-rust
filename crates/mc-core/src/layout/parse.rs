//! Checking layouts that come from outside: a saved one from `tabs.json`, a live one from the
//! renderer. Both read an unchecked JSON value field by field, as the TypeScript did, so that what
//! is repaired and what is refused stays the same.

use std::borrow::Cow;
use std::collections::HashSet;

use serde_json::Value;

use crate::jsstr::longer16;
use crate::types::{Dir, Group, Layout, Node, SavedLayout, SavedNode, Split};

use super::tree::{groups_of, settle};
use super::{clamp_ratio, MAX_DEPTH};

/// The longest node or tab id the renderer may send, in UTF-16 units.
const ID_MAX: usize = 64;

/// Checks a saved layout read from a file. None for anything malformed; a bad ratio or focus is
/// repaired rather than fatal.
pub fn parse_saved_layout(raw: &Value) -> Option<SavedLayout> {
    struct Parse {
        seen: HashSet<i64>,
        groups: u64,
    }
    impl Parse {
        fn visit(&mut self, value: Option<&Value>, depth: usize) -> Option<SavedNode> {
            let node = value?.as_object()?;
            if depth > MAX_DEPTH {
                return None;
            }
            if let Some(tabs) = node.get("tabs").and_then(Value::as_array) {
                if tabs.is_empty() {
                    return None;
                }
                let mut out = Vec::with_capacity(tabs.len());
                for t in tabs {
                    let t = js_integer(Some(t)).filter(|t| *t >= 0)?;
                    if !self.seen.insert(t) {
                        return None;
                    }
                    out.push(t);
                }
                self.groups += 1;
                return Some(SavedNode::Group { tabs: out, active: js_integer(node.get("active")).unwrap_or(-1) });
            }
            let dir = dir_of_value(node.get("dir"))?;
            let first = self.visit(node.get("first"), depth + 1);
            let second = self.visit(node.get("second"), depth + 1);
            Some(SavedNode::Split {
                dir,
                ratio: clamp_ratio(js_to_number(node.get("ratio"))),
                first: Box::new(first?),
                second: Box::new(second?),
            })
        }
    }
    let mut parse = Parse { seen: HashSet::new(), groups: 0 };
    let root = parse.visit(raw.get("root"), 0)?;
    let focused = js_integer(raw.get("focused"))
        .filter(|f| *f >= 0 && (*f as u64) < parse.groups)
        .and_then(|f| u32::try_from(f).ok())
        .unwrap_or(0);
    Some(SavedLayout { root, focused })
}

/// Checks a layout that arrives from another process. None for anything malformed: ids must be
/// 1 to 64 UTF-16 units, node ids and tab ids unique, the tree at most [`MAX_DEPTH`] deep. What
/// can be repaired is: a shown tab that is not in its group (the last tab is shown), a focus on a
/// group that does not exist (the first group gets it), empty groups (only a lone root may be
/// empty), and `seq`, which is raised past every number at the end of a node id.
pub fn parse_layout(raw: &Value) -> Option<Layout> {
    struct Parse {
        node_ids: HashSet<String>,
        tab_ids: HashSet<String>,
    }
    impl Parse {
        fn visit(&mut self, value: Option<&Value>, depth: usize) -> Option<Node> {
            let node = value?.as_object()?;
            if depth > MAX_DEPTH {
                return None;
            }
            let id = id_of(node.get("id"))?;
            if !self.node_ids.insert(id.to_string()) {
                return None;
            }
            match node.get("kind").and_then(Value::as_str) {
                Some("group") => {
                    let raw_tabs = node.get("tabs")?.as_array()?;
                    let mut tabs = Vec::with_capacity(raw_tabs.len());
                    for t in raw_tabs {
                        let t = id_of(Some(t))?;
                        if !self.tab_ids.insert(t.to_string()) {
                            return None;
                        }
                        tabs.push(t.to_string());
                    }
                    let active = id_of(node.get("active"))
                        .filter(|a| tabs.iter().any(|t| t == a))
                        .map(str::to_string)
                        .or_else(|| tabs.last().cloned());
                    Some(Node::Group(Group { id: id.to_string(), tabs, active }))
                }
                Some("split") => {
                    let dir = dir_of_value(node.get("dir"))?;
                    let first = self.visit(node.get("first"), depth + 1);
                    let second = self.visit(node.get("second"), depth + 1);
                    Some(Node::Split(Split {
                        id: id.to_string(),
                        dir,
                        ratio: clamp_ratio(js_to_number(node.get("ratio"))),
                        first: Box::new(first?),
                        second: Box::new(second?),
                    }))
                }
                _ => None,
            }
        }
    }
    let mut parse = Parse { node_ids: HashSet::new(), tab_ids: HashSet::new() };
    let root = parse.visit(raw.get("root"), 0)?;
    let focused = {
        let all = groups_of(&root);
        id_of(raw.get("focused")).filter(|f| all.iter().any(|g| g.id == *f)).unwrap_or(all[0].id.as_str()).to_string()
    };
    // new node ids are numbered from `seq`, so it must be past every number in use
    let used = parse.node_ids.iter().map(|id| trailing_number(id)).max().unwrap_or(0);
    let seq = used.max(js_integer(raw.get("seq")).filter(|s| *s >= 0).map_or(0, |s| s as u64));
    let layout = Layout { root, focused, seq };
    // an empty group is only allowed as the whole layout
    let settled = match settle(&layout, Cow::Borrowed(&layout.root), &layout.focused, layout.seq) {
        Cow::Owned(next) => Some(next),
        Cow::Borrowed(_) => None,
    };
    Some(settled.unwrap_or(layout))
}

/// `isId`: a text of 1 to 64 UTF-16 units.
fn id_of(value: Option<&Value>) -> Option<&str> {
    value?.as_str().filter(|s| !s.is_empty() && !longer16(s, ID_MAX))
}

fn dir_of_value(value: Option<&Value>) -> Option<Dir> {
    match value?.as_str()? {
        "row" => Some(Dir::Row),
        "column" => Some(Dir::Column),
        _ => None,
    }
}

/// `Number(/\d+$/.exec(id)?.[0] ?? 0)`: the number an id ends with, 0 if none. Beyond `u64` it
/// saturates (JavaScript would carry on with an imprecise float).
fn trailing_number(id: &str) -> u64 {
    let digits = id.len() - id.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    id[id.len() - digits..].bytes().fold(0u64, |n, d| n.saturating_mul(10).saturating_add(u64::from(d - b'0')))
}

/// The value when `Number.isInteger(value)` holds: a JSON number without a fraction (`3.0` too).
/// Integers beyond `i64` saturate.
fn js_integer(value: Option<&Value>) -> Option<i64> {
    let n = value?.as_number()?;
    if let Some(i) = n.as_i64() {
        return Some(i);
    }
    let f = n.as_f64()?;
    (f.is_finite() && f.fract() == 0.0).then_some(f as i64)
}

/// `Number(value)` of JavaScript, for a JSON value (`None` is `undefined`).
fn js_to_number(value: Option<&Value>) -> f64 {
    match value {
        None | Some(Value::Object(_)) => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => string_to_number(s),
        // through `String(array)`: "" for none, the element's text for one, "a,b" (NaN) for more
        Some(Value::Array(items)) => match items.as_slice() {
            [] => 0.0,
            [Value::Bool(_)] => f64::NAN,
            [one] => js_to_number(Some(one)),
            _ => f64::NAN,
        },
    }
}

/// `Number(text)`: blank is 0, surrounding white space is ignored, `0x`/`0o`/`0b` are integers,
/// anything else that is not a decimal number is NaN.
fn string_to_number(s: &str) -> f64 {
    const JS_SPACE: &[char] = &[
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
        '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}',
        '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}',
    ];
    let s = s.trim_matches(JS_SPACE);
    if s.is_empty() {
        return 0.0;
    }
    match s {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = match s.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &s[2..];
        if digits.is_empty() {
            return f64::NAN;
        }
        return digits
            .chars()
            .try_fold(0f64, |n, c| c.to_digit(radix).map(|d| n * f64::from(radix) + f64::from(d)))
            .unwrap_or(f64::NAN);
    }
    // the decimal literal of JavaScript: no "inf", "nan" or other words that Rust would read
    let decimal = s
        .strip_prefix(['+', '-'])
        .unwrap_or(s)
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if decimal {
        s.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_numbers_as_javascript_does() {
        assert_eq!(js_to_number(Some(&json!(0.3))), 0.3);
        assert_eq!(js_to_number(Some(&json!("0.3"))), 0.3);
        assert_eq!(js_to_number(Some(&json!(" 0.3\n"))), 0.3);
        assert_eq!(js_to_number(Some(&json!(""))), 0.0);
        assert_eq!(js_to_number(Some(&json!(null))), 0.0);
        assert_eq!(js_to_number(Some(&json!(true))), 1.0);
        assert_eq!(js_to_number(Some(&json!([]))), 0.0);
        assert_eq!(js_to_number(Some(&json!(["0.3"]))), 0.3);
        assert_eq!(js_to_number(Some(&json!("0x10"))), 16.0);
        assert_eq!(js_to_number(Some(&json!(".5"))), 0.5);
        assert!(js_to_number(None).is_nan());
        assert!(js_to_number(Some(&json!("inf"))).is_nan());
        assert!(js_to_number(Some(&json!("1,2"))).is_nan());
        assert!(js_to_number(Some(&json!([1, 2]))).is_nan());
        assert!(js_to_number(Some(&json!({}))).is_nan());
        assert_eq!(js_integer(Some(&json!(3.0))), Some(3));
        assert_eq!(js_integer(Some(&json!(3.5))), None);
        assert_eq!(js_integer(Some(&json!("3"))), None);
        assert_eq!(trailing_number("s12"), 12);
        assert_eq!(trailing_number("g007"), 7);
        assert_eq!(trailing_number("node"), 0);
    }
}
