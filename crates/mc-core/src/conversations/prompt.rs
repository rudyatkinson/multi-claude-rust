//! `cleanPrompt`: a message as the user would recognize it.

use std::sync::LazyLock;

use regex::Regex;

use crate::jsstr::cut16;
use crate::usage::{is_js_space, js_trim};

/// A message is cut to this many UTF-16 units.
pub const PROMPT_CHARS: usize = 300;

/// Blocks Claude Code wraps around what it adds to a message itself.
const ADDED: [&str; 7] = [
    "system-reminder",
    "local-command-stdout",
    "local-command-stderr",
    "local-command-caveat",
    "command-message",
    "bash-stdout",
    "bash-stderr",
];

static COMMAND_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<command-name>([\s\S]*?)</command-name>").unwrap());
static COMMAND_ARGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<command-args>([\s\S]*?)</command-args>").unwrap());

/// `<(a|b|...)>[\s\S]*?<\/\1>`. The regex crate has no back references, so each name is an
/// alternative of its own; only one of them can start at a given place, so the matches are the same.
static ADDED_BLOCKS: LazyLock<Regex> = LazyLock::new(|| {
    let alternatives: Vec<String> = ADDED.iter().map(|name| format!(r"<{name}>[\s\S]*?</{name}>")).collect();
    Regex::new(&alternatives.join("|")).unwrap()
});

/// A block whose end is missing because the text was cut off: from its start to the end.
static ADDED_OPENING: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"<(?:{})>[\s\S]*$", ADDED.join("|"))).unwrap());

/// A message as the user would recognize it: a slash command as `/name args`, otherwise the text
/// without the blocks Claude Code added, on one line, at most [`PROMPT_CHARS`] long. `None` when
/// nothing typed is left.
pub fn clean_prompt(text: &str) -> Option<String> {
    let command = COMMAND_NAME.captures(text).map(|c| js_trim(c.get(1).map_or("", |m| m.as_str()))).filter(|c| !c.is_empty());
    let typed = match command {
        Some(command) => {
            let args = COMMAND_ARGS.captures(text).and_then(|c| c.get(1)).map_or("", |m| m.as_str());
            format!("{command} {args}")
        }
        None => {
            let without = ADDED_BLOCKS.replace_all(text, " ");
            ADDED_OPENING.replace(&without, " ").into_owned()
        }
    };
    let line = one_line(&typed);
    (!line.is_empty()).then(|| cut16(&line, PROMPT_CHARS).to_string())
}

/// `text.replace(/\s+/g, ' ').trim()`.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars() {
        if is_js_space(c) {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- cleanPrompt (test/conversations.test.ts) ----------

    #[test]
    fn keeps_typed_text_on_one_line() {
        assert_eq!(clean_prompt("Fix the  login\nbug\tplease").as_deref(), Some("Fix the login bug please"));
    }

    #[test]
    fn shows_a_slash_command_the_way_it_was_typed() {
        assert_eq!(
            clean_prompt("<command-name>/review</command-name>\n<command-message>review</command-message>\n<command-args>the last commit</command-args>")
                .as_deref(),
            Some("/review the last commit")
        );
        assert_eq!(clean_prompt("<command-name>/usage</command-name><command-args></command-args>").as_deref(), Some("/usage"));
    }

    #[test]
    fn drops_what_claude_code_added_around_a_message() {
        assert_eq!(
            clean_prompt("<system-reminder>context</system-reminder>Add a test<local-command-stdout>ok</local-command-stdout>").as_deref(),
            Some("Add a test")
        );
        assert_eq!(clean_prompt("<local-command-caveat>Caveat: generated</local-command-caveat>"), None);
        assert_eq!(clean_prompt("   "), None);
    }

    #[test]
    fn cuts_very_long_text() {
        assert_eq!(clean_prompt(&"x".repeat(5000)).map(|p| p.len()), Some(300));
    }

    #[test]
    fn drops_a_block_whose_end_was_cut_off() {
        assert_eq!(clean_prompt("Look at this<bash-stdout>half of the out").as_deref(), Some("Look at this"));
        // a closing tag of another block does not end one
        assert_eq!(clean_prompt("a<bash-stdout>x</bash-stderr>b").as_deref(), Some("a"));
    }

    #[test]
    fn cuts_at_utf16_units() {
        let prompt = clean_prompt(&"😀".repeat(200)).unwrap();
        assert_eq!(crate::jsstr::len16(&prompt), 300);
    }
}
