//! Dropped paths as terminal input (`src/shared/paths.ts`).

/// A file path as it is typed into a terminal: bare when it is plain, quoted when it has spaces or
/// other characters a command line would split or interpret (`windows`: cmd and PowerShell rules,
/// else a POSIX shell's). `None` for an empty path or one with control characters, which must not
/// reach a terminal as input.
pub fn quote_path(path: &str, windows: bool) -> Option<String> {
    if path.is_empty() || path.chars().any(|c| c < ' ' || c == '\u{7f}') {
        return None;
    }
    let plain = |extra: &[char]| path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/') || extra.contains(&c));
    if windows {
        // a Windows path cannot contain a double quote, so wrapping it in them is always safe
        return Some(if plain(&[':', '\\']) { path.to_string() } else { format!("\"{path}\"") });
    }
    Some(if plain(&[]) { path.to_string() } else { format!("'{}'", path.replace('\'', r"'\''")) })
}

/// The text that dropping these files on a terminal types: their paths, separated and followed by
/// a space. Paths [`quote_path`] refuses are left out; `None` when none is left (the TypeScript
/// returned an empty string).
pub fn paths_as_input<S: AsRef<str>>(paths: &[S], windows: bool) -> Option<String> {
    let quoted: Vec<String> = paths.iter().filter_map(|p| quote_path(p.as_ref(), windows)).collect();
    (!quoted.is_empty()).then(|| quoted.join(" ") + " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- quotePath (test/conversations.test.ts) ----------

    #[test]
    fn leaves_a_plain_path_alone_and_quotes_one_a_command_line_would_split() {
        assert_eq!(quote_path(r"C:\code\app\main.ts", true).as_deref(), Some(r"C:\code\app\main.ts"));
        assert_eq!(quote_path(r"C:\My Projects\app (v2)\a&b.png", true).as_deref(), Some(r#""C:\My Projects\app (v2)\a&b.png""#));
        assert_eq!(quote_path(r"C:\Kullanıcılar\Çağrı\ekran.png", true).as_deref(), Some(r#""C:\Kullanıcılar\Çağrı\ekran.png""#));
        assert_eq!(quote_path("/home/me/code/app.ts", false).as_deref(), Some("/home/me/code/app.ts"));
        assert_eq!(quote_path("/home/me/it's here/a b.txt", false).as_deref(), Some(r"'/home/me/it'\''s here/a b.txt'"));
        // a backslash and a colon are plain only on Windows
        assert_eq!(quote_path(r"a\b:c", false).as_deref(), Some(r"'a\b:c'"));
    }

    #[test]
    fn refuses_a_path_with_control_characters() {
        assert_eq!(quote_path("C:\\code\\a\nrm -rf", true), None);
        assert_eq!(quote_path("/tmp/a\u{1b}[2J", false), None);
        assert_eq!(quote_path("/tmp/a\u{7f}", false), None);
        assert_eq!(quote_path("", true), None);
    }

    #[test]
    fn joins_dropped_files_into_one_line_that_ends_ready_for_more_typing() {
        assert_eq!(paths_as_input(&[r"C:\a\one.png", r"C:\a b\two.png"], true).as_deref(), Some(r#"C:\a\one.png "C:\a b\two.png" "#));
        assert_eq!(paths_as_input(&["C:\\bad\nname"], true), None);
        assert_eq!(paths_as_input::<&str>(&[], true), None);
        assert_eq!(paths_as_input(&["C:\\bad\nname", r"C:\ok"], true).as_deref(), Some(r"C:\ok "));
    }
}
