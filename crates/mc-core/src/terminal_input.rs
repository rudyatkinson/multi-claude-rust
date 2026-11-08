//! `is_typing` (`src/shared/terminalInput.ts`).

use std::sync::LazyLock;

use regex::Regex;

// What a terminal sends without anyone touching the keyboard: that it gained or lost the focus,
// where the mouse is, and its answers to what the program in it asked. JavaScript's `\d` is
// ASCII, hence `[0-9]`; `[\s\S]` is any character, hence `(?s:.)`.
static REPORTS: LazyLock<Regex> = LazyLock::new(|| {
    let alternatives = [
        r"\x1b\[[IO]",                         // focus in, focus out
        r"\x1b\[<[0-9]+;[0-9]+;[0-9]+[Mm]",    // mouse
        r"\x1b\[M(?s:.){3}",                   // mouse, the old encoding
        r"\x1b\[[0-9]+;[0-9]+;[0-9]+M",        // mouse, the urxvt encoding
        r"\x1b\[\??[0-9]+;[0-9]+R",            // cursor position
        r"\x1b\[[?>=][0-9;]*c",                // device attributes
        r"\x1b\[\??[0-9]+n",                   // device status
        r"\x1b\[\??[0-9;]+\$y",                // state of a mode
        r"\x1b\[\?[0-9;]*u",                   // keyboard protocol
        r"\x1b\[[0-9;]+t",                     // window size
        r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", // colours and other OSC answers
        r"\x1bP[^\x1b]*\x1b\\",                // version and settings
    ];
    Regex::new(&alternatives.join("|")).unwrap()
});

/// Whether what a terminal sends to its program holds anything a person typed or pasted. The
/// reports above travel the same way, and a tab that is merely clicked sends some of them.
pub fn is_typing(data: &str) -> bool {
    !REPORTS.replace_all(data, "").is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: &str = "\x1b";

    #[test]
    fn takes_keys_and_pastes_for_typing() {
        let keys = [
            "a".to_string(),
            "\r".into(),
            " ".into(),
            "\x03".into(),
            ESC.into(),
            format!("{ESC}[A"),
            format!("{ESC}OP"),
            format!("{ESC}[15~"),
            format!("{ESC}\r"),
            format!("{ESC}[200~some text{ESC}[201~"),
            "ış".into(),
        ];
        for data in keys {
            assert!(is_typing(&data), "{data:?}");
        }
    }

    #[test]
    fn does_not_take_what_the_terminal_reports_by_itself_for_typing() {
        let reports = [
            format!("{ESC}[I"),
            format!("{ESC}[O"),
            format!("{ESC}[<0;12;7M"),
            format!("{ESC}[<0;12;7m"),
            format!("{ESC}[<64;40;3M"),
            format!("{ESC}[M !!"),
            format!("{ESC}[32;12;7M"),
            format!("{ESC}[24;80R"),
            format!("{ESC}[?1;2c"),
            format!("{ESC}[>0;276;0c"),
            format!("{ESC}[0n"),
            format!("{ESC}[?2026;2$y"),
            format!("{ESC}[?0u"),
            format!("{ESC}[8;30;120t"),
            format!("{ESC}]11;rgb:1414/1414/1313{ESC}\\"),
            format!("{ESC}]10;rgb:e8e8/e6e6/dcdc\x07"),
            format!("{ESC}P>|xterm.js(6.0.0){ESC}\\"),
        ];
        for data in &reports {
            assert!(!is_typing(data), "{data:?}");
        }
        assert!(!is_typing(&reports.concat()));
        assert!(!is_typing(""));
    }

    #[test]
    fn finds_a_key_between_reports() {
        assert!(is_typing(&format!("{ESC}[Iy{ESC}[O")));
        assert!(is_typing(&format!("{ESC}[<0;12;7M\r")));
    }

    #[test]
    fn digits_are_ascii_as_in_javascript() {
        // an Arabic-Indic digit is not one to the terminal
        assert!(is_typing(&format!("{ESC}[\u{0661}n")));
    }
}
