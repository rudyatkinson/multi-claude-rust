//! `parseHead` and `parseTail`: what the beginning and the end of a conversation file say.

use serde_json::{Map, Value};

use super::json::{field_of_cut, whole_cwd};
use super::prompt::clean_prompt;
use crate::jsstr::cut16;
use crate::usage::js_trim;

/// Claude Code's title is cut to this many UTF-16 units.
pub const AI_TITLE_CHARS: usize = 200;
/// How far past a `"cwd":"` its value is looked for at the end of a file, in UTF-16 units.
const CWD_WINDOW: usize = 4096;

/// What the beginning of a conversation file says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Head {
    /// The folder, as the first record that has one names it.
    pub cwd: Option<String>,
    /// The first message the user typed, cleaned with [`clean_prompt`].
    pub first_prompt: Option<String>,
}

/// What the end of a conversation file says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tail {
    /// Claude Code's latest title, trimmed and cut to [`AI_TITLE_CHARS`].
    pub title: Option<String>,
    /// The last message, cleaned with [`clean_prompt`].
    pub last_prompt: Option<String>,
    /// The folder, as the last records name it; for a file whose beginning did not give it.
    pub cwd: Option<String>,
}

/// `!value` for an optional string of the TypeScript: missing or empty.
fn unset(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(str::is_empty)
}

/// The typed message of a record: not what Claude Code injected, and not what a subagent was told.
fn prompt_of(rec: &Map<String, Value>) -> Option<String> {
    let flagged = |key: &str| rec.get(key) == Some(&Value::Bool(true));
    if rec.get("type").and_then(Value::as_str) != Some("user") || flagged("isMeta") || flagged("isSidechain") {
        return None;
    }
    match rec.get("message").and_then(|m| m.get("content"))? {
        Value::String(text) => clean_prompt(text),
        // a record with only tool results has no text block
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|b| (b.get("type").and_then(Value::as_str) == Some("text")).then(|| b.get("text").and_then(Value::as_str)).flatten())
            .and_then(clean_prompt),
        _ => None,
    }
}

/// The folder and the first typed message, from the beginning of a conversation file. When
/// `truncated`, the bytes end in the middle of a line: a first message that is itself longer than
/// what was read is recovered from that piece. Bytes that are not UTF-8 read as U+FFFD, as
/// `Buffer.toString('utf8')` read them.
pub fn parse_head(bytes: &[u8], truncated: bool) -> Head {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    let cut = if truncated { lines.pop() } else { None };
    let mut info = Head::default();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Ok(Value::Object(rec)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if unset(&info.cwd) {
            if let Some(Value::String(cwd)) = rec.get("cwd") {
                info.cwd = Some(cwd.clone());
            }
        }
        if unset(&info.first_prompt) {
            info.first_prompt = prompt_of(&rec);
        }
        if !unset(&info.cwd) && !unset(&info.first_prompt) {
            return info;
        }
    }
    if let Some(cut) = cut.filter(|c| !c.is_empty()) {
        if cut.contains(r#""type":"user""#) && !cut.contains(r#""isMeta":true"#) && !cut.contains(r#""isSidechain":true"#) {
            if unset(&info.first_prompt) {
                let text = field_of_cut(cut, "content").or_else(|| field_of_cut(cut, "text"));
                info.first_prompt = text.and_then(|t| clean_prompt(&t));
            }
            if unset(&info.cwd) {
                info.cwd = whole_cwd(cut);
            }
        }
    }
    info
}

/// The last complete record of a type in a piece of a file. `None` also when the line it is on
/// does not parse: cut at the start of the piece, or still being written.
fn last_record(text: &str, kind: &str) -> Option<Value> {
    let at = text.rfind(&format!("\"type\":\"{kind}\""))?;
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    serde_json::from_str(&text[start..end]).ok()
}

/// Claude Code's latest title, the last message and the folder, from the end of a conversation
/// file. The bytes are taken as they are; [`read_tail`](super::read_tail) starts them at the first
/// whole line.
pub fn parse_tail(bytes: &[u8]) -> Tail {
    const CWD: &str = "\"cwd\":\"";
    let text = String::from_utf8_lossy(bytes);
    let title = last_record(&text, "ai-title")
        .and_then(|rec| rec.get("aiTitle").and_then(Value::as_str).map(js_trim).filter(|t| !t.is_empty()).map(|t| cut16(t, AI_TITLE_CHARS).to_string()));
    let last_prompt = last_record(&text, "last-prompt").and_then(|rec| rec.get("lastPrompt").and_then(Value::as_str).and_then(clean_prompt));
    // the folder is on most records; the last one that has it whole will do
    let mut cwd = None;
    let mut before = text.len();
    while cwd.is_none() {
        let Some(at) = text[..before].rfind(CWD) else {
            break;
        };
        cwd = whole_cwd(cut16(&text[at..], CWD_WINDOW));
        // `lastIndexOf(CWD, at - 1)`: an earlier match may overlap this one
        before = at + CWD.len() - 1;
    }
    Tail { title, last_prompt, cwd }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub const ID: &str = "2bf04019-6df2-46c6-93d8-91c02cde383c";
    pub const APP: &str = r"C:\code\app";

    /// A JSON string, as `JSON.stringify` writes it.
    pub fn js(text: &str) -> String {
        serde_json::to_string(text).unwrap()
    }

    /// `user(content, extra)` of the TypeScript tests, with its key order: `content` is JSON.
    pub fn user_rec(content: &str, sidechain: bool, cwd: &str, meta: bool) -> String {
        let meta = if meta { r#","isMeta":true"# } else { "" };
        format!(
            r#"{{"parentUuid":null,"isSidechain":{sidechain},"type":"user","message":{{"role":"user","content":{content}}},"cwd":{},"timestamp":"2026-10-07T12:00:00Z"{meta}}}"#,
            js(cwd)
        )
    }

    /// `user('text')`.
    pub fn user(text: &str) -> String {
        user_rec(&js(text), false, APP, false)
    }

    pub fn title(text: &str) -> String {
        format!(r#"{{"type":"ai-title","aiTitle":{}}}"#, js(text))
    }

    pub fn last_prompt(text: &str) -> String {
        format!(r#"{{"type":"last-prompt","lastPrompt":{}}}"#, js(text))
    }

    fn head(text: &str, complete: bool) -> Head {
        parse_head(text.as_bytes(), !complete)
    }

    fn head_of(cwd: Option<&str>, first_prompt: Option<&str>) -> Head {
        Head { cwd: cwd.map(str::to_string), first_prompt: first_prompt.map(str::to_string) }
    }

    // ---------- parseHead (test/conversations.test.ts) ----------

    #[test]
    fn finds_the_folder_and_the_first_typed_message() {
        let text = [format!(r#"{{"type":"mode","mode":"default","sessionId":"{ID}"}}"#), user("Fix the login bug"), user("second message")].join("\n") + "\n";
        assert_eq!(head(&text, true), head_of(Some(APP), Some("Fix the login bug")));
    }

    #[test]
    fn skips_what_the_user_did_not_type() {
        let text = [
            user_rec(&js("<local-command-caveat>Caveat</local-command-caveat>"), false, APP, true),
            user_rec(&js("do the subtask"), true, APP, false),
            user_rec(r#"[{"type":"tool_result","tool_use_id":"t1","content":"output"}]"#, false, APP, false),
            user_rec(r#"[{"type":"text","text":"Now the real question"}]"#, false, APP, false),
        ]
        .join("\n");
        assert_eq!(head(&text, true).first_prompt.as_deref(), Some("Now the real question"));
    }

    #[test]
    fn takes_a_slash_command_as_the_first_message() {
        assert_eq!(head(&user("<command-name>/init</command-name><command-args></command-args>"), true).first_prompt.as_deref(), Some("/init"));
    }

    #[test]
    fn recovers_a_first_message_that_is_longer_than_what_was_read() {
        let whole = user(&("Summarize this document: ".to_string() + &r#"lorem "ipsum" \ dolor "#.repeat(2000)));
        // the piece ends in the middle of the record, possibly inside an escape
        for length in [600, 601, 602, 603, 1500] {
            let text = format!("{}\n{}", r#"{"type":"mode"}"#, &whole[..length]);
            let first = head(&text, false).first_prompt.unwrap_or_default();
            assert!(first.starts_with(r#"Summarize this document: lorem "ipsum" \ dolor"#), "cut at {length}: {first}");
        }
    }

    #[test]
    fn takes_the_folder_from_a_cut_record_only_when_all_of_it_is_there() {
        let record = format!(r#"{{"type":"user","cwd":{},"message":{{"role":"user","content":"hello there, this goes on"}}}}"#, js(APP));
        assert_eq!(head(&record[..record.len() - 14], false), head_of(Some(APP), Some("hello there, t")));
        // half a path would be another folder
        let cut_in_path = format!(r#"{{"type":"user","message":{{"role":"user","content":"hello"}},"cwd":{}}}"#, js(APP));
        assert_eq!(head(&cut_in_path[..cut_in_path.len() - 4], false), head_of(None, Some("hello")));
    }

    #[test]
    fn does_not_mistake_a_cut_record_for_a_typed_message_when_it_is_not_one() {
        let injected = r#"{"type":"user","isMeta":true,"message":{"content":"injected"}}"#;
        assert_eq!(head(&injected[..injected.len() - 5], false), Head::default());
        // the marker that says so may be past the cut: the text itself gives it away
        let caveat = user_rec(&js(&format!("<local-command-stdout>{}</local-command-stdout>", "y".repeat(500))), false, APP, true);
        assert_eq!(head(&caveat[..300], false), Head::default());
    }

    #[test]
    fn does_not_trip_over_broken_lines() {
        let text = format!("not json\n{}\n{{\"type\":", user("still found"));
        assert_eq!(head(&text, true).first_prompt.as_deref(), Some("still found"));
        assert_eq!(head("", true), Head::default());
        assert_eq!(head("", false), Head::default());
    }

    #[test]
    fn reads_bytes_that_are_not_utf8() {
        let mut bytes = user("caf\u{e9}").into_bytes();
        bytes.extend_from_slice(b"\n\xff\xfe\n");
        assert_eq!(parse_head(&bytes, false).first_prompt.as_deref(), Some("caf\u{e9}"));
    }

    // ---------- parseTail (test/conversations.test.ts) ----------

    fn tail(text: &str) -> Tail {
        parse_tail(text.as_bytes())
    }

    #[test]
    fn takes_the_latest_title_and_the_last_message() {
        let text = [
            format!(r#"{{"type":"ai-title","aiTitle":"Old title","sessionId":"{ID}"}}"#),
            format!(r#"{{"type":"last-prompt","lastPrompt":"first question","sessionId":"{ID}"}}"#),
            format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}}]}}}}"#, js(r#"answer mentioning "type":"ai-title" in prose"#)),
            format!(r#"{{"type":"ai-title","aiTitle":"Fix login redirect","sessionId":"{ID}"}}"#),
            format!(r#"{{"type":"last-prompt","lastPrompt":"and add a test","sessionId":"{ID}"}}"#),
        ]
        .join("\n");
        assert_eq!(tail(&text), Tail { title: Some("Fix login redirect".into()), last_prompt: Some("and add a test".into()), cwd: None });
    }

    #[test]
    fn has_no_title_for_a_conversation_without_one_or_when_the_last_record_is_still_being_written() {
        assert_eq!(tail(&user("hello")), Tail { cwd: Some(APP.into()), ..Tail::default() });
        assert_eq!(tail(r#"{"type":"ai-title","aiTitle":"Half wri"#), Tail::default());
    }

    #[test]
    fn names_the_folder_from_the_last_record_that_has_it_whole() {
        let cut = format!(r#"{{"type":"user","cwd":{}}}"#, js(r"E:\cut off somewhere"));
        let text = [user("one"), user_rec(&js("two"), false, r"D:\moved", false), cut[..32].to_string()].join("\n");
        assert_eq!(tail(&text).cwd.as_deref(), Some(r"D:\moved"));
    }

    #[test]
    fn trims_and_cuts_the_title_and_cleans_the_last_message() {
        let text = [title(&format!("  {}  ", "t".repeat(250))), last_prompt("<command-name>/clear</command-name>")].join("\n");
        let got = tail(&text);
        assert_eq!(got.title.map(|t| t.len()), Some(200));
        assert_eq!(got.last_prompt.as_deref(), Some("/clear"));
        assert_eq!(tail(&title("   ")).title, None);
    }

    #[test]
    fn finds_a_folder_whose_marker_overlaps_a_later_one() {
        // `lastIndexOf` finds the match that ends inside the one after it, not only the "A" before
        assert_eq!(tail(r#""cwd":"A","cwd":"cwd":"half"#).cwd.as_deref(), Some("cwd"));
        assert_eq!(tail(r#""cwd":"A","cwd":"half"#).cwd.as_deref(), Some("A"));
    }
}
