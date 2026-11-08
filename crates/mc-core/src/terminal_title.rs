//! `parse_terminal_title` (`src/shared/terminalTitle.ts`).

use serde::{Deserialize, Serialize};

use crate::turn::js_trim;

/// What a session's terminal title says about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabStatus {
    /// The conversation's topic as Claude Code named it; `None` until it has one.
    pub topic: Option<String>,
    /// Claude is working, as opposed to waiting for input.
    pub busy: bool,
}

// Claude Code writes its terminal title as "<mark> <text>": ✳ while it waits, and ◐ / ◑
// alternating about once a second while it works.
const IDLE_MARK: char = '\u{2733}';
const BUSY_MARKS: [char; 2] = ['\u{25D0}', '\u{25D1}'];
// the title before a conversation has a topic
const DEFAULT_TEXT: &str = "claude code";

/// `None` for a title that does not carry Claude Code's mark. Those come from elsewhere: the bare
/// process name before Claude Code has drawn anything, or a program it runs in the same console.
pub fn parse_terminal_title(raw: &str) -> Option<TabStatus> {
    let trimmed = js_trim(raw);
    let mark = trimmed.chars().next()?;
    let busy = BUSY_MARKS.contains(&mark);
    if !busy && mark != IDLE_MARK {
        return None;
    }
    // U+FE0F may follow the mark when the terminal title is written with emoji presentation
    let rest = &trimmed[mark.len_utf8()..];
    let text = js_trim(rest.strip_prefix('\u{FE0F}').unwrap_or(rest));
    let topic = (!text.is_empty() && text.to_lowercase() != DEFAULT_TEXT).then(|| text.to_string());
    Some(TabStatus { topic, busy })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(topic: Option<&str>, busy: bool) -> Option<TabStatus> {
        Some(TabStatus { topic: topic.map(str::to_string), busy })
    }

    #[test]
    fn reads_the_topic_of_a_waiting_session() {
        assert_eq!(parse_terminal_title("✳ translate-ui-refresh-feature"), status(Some("translate-ui-refresh-feature"), false));
    }

    #[test]
    fn treats_either_half_circle_mark_as_working() {
        assert_eq!(parse_terminal_title("◐ Fix the login flow"), status(Some("Fix the login flow"), true));
        assert_eq!(parse_terminal_title("◑ Fix the login flow"), status(Some("Fix the login flow"), true));
    }

    #[test]
    fn has_no_topic_for_the_default_title_a_bare_mark_or_an_empty_title() {
        assert_eq!(parse_terminal_title("✳ Claude Code"), status(None, false));
        assert_eq!(parse_terminal_title("◐ Claude Code"), status(None, true));
        assert_eq!(parse_terminal_title("✳ CLAUDE CODE"), status(None, false));
        assert_eq!(parse_terminal_title("✳"), status(None, false));
    }

    #[test]
    fn accepts_the_mark_with_an_emoji_variation_selector() {
        assert_eq!(parse_terminal_title("✳️  review plan "), status(Some("review plan"), false));
    }

    #[test]
    fn ignores_titles_that_do_not_come_from_claude_code() {
        // the process name before the first draw, and programs started in the same console
        assert_eq!(parse_terminal_title("claude"), None);
        assert_eq!(parse_terminal_title("npm run dev"), None);
        assert_eq!(parse_terminal_title(""), None);
        assert_eq!(parse_terminal_title("   "), None);
    }

    #[test]
    fn serializes_as_the_typescript_did() {
        assert_eq!(
            serde_json::to_value(parse_terminal_title("✳ Claude Code")).unwrap(),
            serde_json::json!({ "topic": null, "busy": false })
        );
    }
}
