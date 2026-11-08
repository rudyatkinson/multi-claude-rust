//! `src/main/workflows/transcript.ts`: the last thing Claude said in a conversation file (one
//! JSON record per line), for taking it as a step's output.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::turn::js_trim;

/// A final answer is near the end of the conversation file; this much of it is read.
pub const TAIL_BYTES: u64 = 1024 * 1024;

/// The text blocks of an assistant record that is not a subagent's, joined by line breaks and
/// trimmed; None when that leaves nothing.
fn text_of(rec: &Value) -> Option<String> {
    // not what a subagent said
    if rec.get("type").and_then(Value::as_str) != Some("assistant") || rec.get("isSidechain") == Some(&Value::Bool(true)) {
        return None;
    }
    let content = rec.get("message")?.get("content")?.as_array()?;
    let parts: Vec<&str> = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect();
    let joined = parts.join("\n");
    let text = js_trim(&joined);
    (!text.is_empty()).then(|| text.to_string())
}

/// The last thing Claude said in a conversation, from the end of its file (the first line of
/// `tail` may be cut off). None when it said nothing there.
pub fn last_assistant_text(tail: &str) -> Option<String> {
    tail.split('\n').rev().find_map(|line| {
        let line = js_trim(line);
        if !line.starts_with('{') {
            return None;
        }
        serde_json::from_str::<Value>(line).ok().and_then(|rec| text_of(&rec))
    })
}

/// [`last_assistant_text`] of the last [`TAIL_BYTES`] of a conversation file, from the first
/// whole line in them on. None when the file cannot be read.
pub fn read_last_answer(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let start = size - size.min(TAIL_BYTES);
    // one byte more tells whether the tail starts at the beginning of a line
    let from = start.saturating_sub(1);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buffer = Vec::new();
    file.take(size - from).read_to_end(&mut buffer).ok()?;
    let whole = if start == 0 {
        &buffer[..]
    } else {
        buffer.iter().position(|&b| b == b'\n').map_or(&[][..], |at| &buffer[at + 1..])
    };
    last_assistant_text(&String::from_utf8_lossy(whole))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    fn record(kind: &str, content: Value) -> Value {
        json!({ "type": kind, "message": { "content": content } })
    }
    fn line(kind: &str, content: Value) -> String {
        record(kind, content).to_string()
    }
    fn said(text: &str) -> Value {
        json!([{ "type": "text", "text": text }])
    }

    #[test]
    fn is_the_text_of_the_last_record_in_which_claude_said_something() {
        let mut sidechain = record("assistant", said("said by a subagent"));
        sidechain["isSidechain"] = json!(true);
        let tail = [
            "ut off record\"}".to_string(),
            line("assistant", said("first answer")),
            line("user", json!("next question")),
            line("assistant", json!([{ "type": "thinking", "thinking": "hm" }])),
            line("assistant", json!([{ "type": "text", "text": "final" }, { "type": "text", "text": "answer" }])),
            line("assistant", json!([{ "type": "tool_use", "name": "Read", "input": {} }])),
            sidechain.to_string(),
            json!({ "type": "system", "subtype": "turn_duration" }).to_string(),
            String::new(),
        ]
        .join("\n");
        assert_eq!(last_assistant_text(&tail).as_deref(), Some("final\nanswer"));
    }

    #[test]
    fn is_nothing_when_claude_has_not_spoken() {
        assert_eq!(last_assistant_text(""), None);
        assert_eq!(last_assistant_text(&line("user", json!("hello"))), None);
        assert_eq!(last_assistant_text(&line("assistant", said("   "))), None);
        assert_eq!(last_assistant_text("not json at all"), None);
    }

    #[test]
    fn is_read_from_the_end_of_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("conversation.jsonl");
        fs::write(&file, format!("{}\n{}\n", line("assistant", said("old")), line("assistant", said("new")))).unwrap();
        assert_eq!(read_last_answer(&file).as_deref(), Some("new"));
        assert_eq!(read_last_answer(&tmp.path().join("missing.jsonl")), None);
    }

    #[test]
    fn starts_at_the_first_whole_line_of_a_long_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("conversation.jsonl");
        // a record that the tail cuts through, and one that fills the tail exactly
        let cut = line("assistant", said(&"x".repeat(TAIL_BYTES as usize)));
        let padded = |pad: usize| {
            let mut rec = record("assistant", said("whole"));
            rec["pad"] = json!("p".repeat(pad));
            rec.to_string()
        };
        let fill = padded(TAIL_BYTES as usize - 1 - padded(0).len());
        assert_eq!(fill.len() as u64 + 1, TAIL_BYTES);
        fs::write(&file, format!("{cut}\n{fill}\n")).unwrap();
        assert_eq!(read_last_answer(&file).as_deref(), Some("whole"));

        // the only answer is in the line that is cut off
        fs::write(&file, format!("{cut}\n{}\n", line("user", json!("next")))).unwrap();
        assert_eq!(read_last_answer(&file), None);
    }
}
