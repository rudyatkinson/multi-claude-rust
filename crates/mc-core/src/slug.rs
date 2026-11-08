//! Account slugs and the checks of an account's fields (`src/main/profiles.ts`), and `isInside` of
//! `src/main/paths.ts`.
//!
//! Lengths are JavaScript `length`s, counted in UTF-16 units (see [`crate::jsstr`]).

use std::path::{Component, Path};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::error::{AppError, AppResult};
use crate::jsstr::longer16;

/// A slug is cut to this many characters.
pub const SLUG_MAX: usize = 32;
/// The slug of the main account; no other account gets it.
pub const RESERVED_SLUG: &str = "main";
/// What a name that has nothing to make a slug from becomes.
pub const FALLBACK_SLUG: &str = "account";
/// An account name, once trimmed, is at most this long.
pub const PROFILE_NAME_MAX: usize = 40;

static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^#[0-9a-f]{6}$").expect("valid regex"));
// e.g. sk-ant-oat01-..., as printed by `claude setup-token`
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^sk-ant-[A-Za-z0-9_-]{16,}$").expect("valid regex"));
/// `/^[^\s@-][^\s@]*@[^\s@]+$/`, with `\s` spelled out as JavaScript reads it (it has U+FEFF and
/// not U+0085, unlike the `\s` of the regex crate).
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    const S: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
    Regex::new(&format!(r"^[^{S}@-][^{S}@]*@[^{S}@]+$")).expect("valid regex")
});

/// The letters of Turkish that NFKD does not take apart into a Latin letter and a mark.
fn turkish(c: char) -> Option<char> {
    Some(match c {
        'ı' | 'İ' => 'i',
        'ş' | 'Ş' => 's',
        'ğ' | 'Ğ' => 'g',
        'ü' | 'Ü' => 'u',
        'ö' | 'Ö' => 'o',
        'ç' | 'Ç' => 'c',
        _ => return None,
    })
}

/// The folder name of an account: Turkish letters folded, accents dropped, lower case, every run of
/// anything but `a-z0-9` one `-`, no `-` at the ends, at most 32 characters; `account` when nothing
/// is left.
pub fn slugify(name: &str) -> String {
    let folded: String = name.chars().map(|c| turkish(c).unwrap_or(c)).collect();
    let ascii: String = folded.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    let mut slug = String::with_capacity(ascii.len());
    let mut in_run = false;
    for c in ascii.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
            in_run = false;
        } else if !in_run {
            slug.push('-');
            in_run = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    // only ASCII is left, so bytes are characters
    let cut = &trimmed[..trimmed.len().min(SLUG_MAX)];
    if cut.is_empty() {
        FALLBACK_SLUG.to_owned()
    } else {
        cut.to_owned()
    }
}

/// The first of `base`, `base-2`, `base-3`, ... that is not `main`, not in `taken` and for which
/// `exists` (a folder of that name is already there) says no.
pub fn free_slug(base: &str, taken: &[String], exists: impl Fn(&str) -> bool) -> String {
    let mut slug = base.to_owned();
    let mut n = 2u32;
    while slug == RESERVED_SLUG || taken.contains(&slug) || exists(&slug) {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

/// The whitespace `String.prototype.trim` removes: WhiteSpace and LineTerminator of ECMAScript.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{D}'
            | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `validName`: the name trimmed, 1 to 40 UTF-16 units.
pub fn profile_name(name: &str) -> AppResult<String> {
    let name = js_trim(name);
    if name.is_empty() || longer16(name, PROFILE_NAME_MAX) {
        return Err(AppError::invalid("Enter an account name (up to 40 characters)"));
    }
    Ok(name.to_owned())
}

/// `validColor`: `#` and six hex digits, either case.
pub fn profile_color(color: &str) -> AppResult<&str> {
    if !COLOR.is_match(color) {
        return Err(AppError::invalid("Invalid color"));
    }
    Ok(color)
}

/// `validToken`: an OAuth token as `claude setup-token` prints it, trimmed.
pub fn oauth_token(token: &str) -> AppResult<String> {
    let value = js_trim(token);
    if !TOKEN.is_match(value) {
        return Err(AppError::invalid(
            "That does not look like a Claude OAuth token. Paste the \"sk-ant-...\" value printed by claude setup-token.",
        ));
    }
    Ok(value.to_owned())
}

/// The email hint of a new account. It is passed to `claude auth login --email`, so it must not be
/// able to pose as an option.
pub fn email_hint(email: &str) -> AppResult<&str> {
    if !EMAIL.is_match(email) {
        return Err(AppError::invalid("Invalid email address"));
    }
    Ok(email)
}

/// The parts of an absolute path with `.` and `..` resolved, without touching the file system;
/// lower case on Windows, where Node compares paths that way.
fn resolved_parts(p: &Path) -> Option<Vec<String>> {
    let abs = std::path::absolute(p).ok()?;
    let mut parts: Vec<String> = Vec::new();
    let mut fixed = 0usize;
    for comp in abs.components() {
        match comp {
            Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().into_owned());
                fixed = parts.len();
            }
            Component::RootDir => {
                parts.push(std::path::MAIN_SEPARATOR_STR.to_owned());
                fixed = parts.len();
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.len() > fixed {
                    parts.pop();
                }
            }
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
        }
    }
    if cfg!(windows) {
        for part in &mut parts {
            *part = part.to_lowercase();
        }
    }
    Some(parts)
}

/// `isInside(root, path)`: true when `path` is strictly inside `root`, after both are resolved
/// against the working directory. Text only; links are not followed.
///
/// Note the order: the path first, then the folder it should be in.
pub fn is_inside(path: &Path, root: &Path) -> bool {
    let (Some(child), Some(parent)) = (resolved_parts(path), resolved_parts(root)) else {
        return false;
    };
    if child.len() <= parent.len() || child[..parent.len()] != parent[..] {
        return false;
    }
    // `path.relative` gives `..x` for a child named `..x`, which the Electron check refused too
    !child[parent.len()].starts_with("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_folds_turkish_letters_and_punctuation() {
        assert_eq!(slugify("İş Hesabı"), "is-hesabi");
        assert_eq!(slugify("  Çağrı / Şirket #2 "), "cagri-sirket-2");
        assert_eq!(slugify("!!!"), "account");
    }

    #[test]
    fn slugify_drops_accents_and_cuts() {
        assert_eq!(slugify("Café Crème"), "cafe-creme");
        assert_eq!(slugify("ÅNGSTRÖM"), "angstrom");
        assert_eq!(slugify(""), "account");
        assert_eq!(slugify("日本"), "account");
        assert_eq!(slugify(&"a".repeat(40)), "a".repeat(32));
        // the cut comes after the trim, as in the Electron version
        assert_eq!(slugify(&format!("{} b", "a".repeat(31))), format!("{}-", "a".repeat(31)));
    }

    #[test]
    fn free_slug_skips_main_taken_and_existing() {
        let taken = vec!["work".to_owned(), "work-2".to_owned()];
        assert_eq!(free_slug("work", &taken, |_| false), "work-3");
        assert_eq!(free_slug("main", &[], |_| false), "main-2");
        assert_eq!(free_slug("home", &[], |s| s == "home"), "home-2");
        assert_eq!(free_slug("home", &[], |_| false), "home");
    }

    #[test]
    fn names() {
        assert_eq!(profile_name("  Work  ").unwrap(), "Work");
        assert!(profile_name("   ").is_err());
        assert_eq!(profile_name("").unwrap_err().to_string(), "Enter an account name (up to 40 characters)");
        assert!(profile_name(&"n".repeat(40)).is_ok());
        assert!(profile_name(&"n".repeat(41)).is_err());
        // 20 emoji are 40 units
        assert!(profile_name(&"😀".repeat(20)).is_ok());
        assert!(profile_name(&"😀".repeat(21)).is_err());
    }

    #[test]
    fn colors() {
        assert!(profile_color("#5b9bd5").is_ok());
        assert!(profile_color("#5B9BD5").is_ok());
        assert_eq!(profile_color("#5b9bd").unwrap_err().to_string(), "Invalid color");
        assert!(profile_color("5b9bd5").is_err());
        assert!(profile_color("#5b9bd5\n").is_err());
    }

    #[test]
    fn tokens() {
        let token = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz";
        assert_eq!(oauth_token(&format!("  {token}  ")).unwrap(), token);
        assert!(oauth_token("hunter2").unwrap_err().to_string().contains("OAuth token"));
        assert!(oauth_token("sk-ant-short").is_err());
        assert!(oauth_token("sk-ant-oat01-abcdefghijkl mnop").is_err());
    }

    #[test]
    fn emails() {
        assert!(email_hint("me@example.com").is_ok());
        assert_eq!(email_hint("-me@example.com").unwrap_err().to_string(), "Invalid email address");
        assert!(email_hint("me example@x").is_err());
        assert!(email_hint("me@").is_err());
        assert!(email_hint("@x").is_err());
        assert!(email_hint("me\u{feff}@x").is_err());
    }

    #[test]
    fn is_inside_accepts_only_real_children() {
        let root = Path::new("/a/profiles");
        assert!(is_inside(Path::new("/a/profiles/work"), root));
        assert!(!is_inside(Path::new("/a/profiles"), root));
        assert!(!is_inside(Path::new("/a/profiles/../other"), root));
        assert!(!is_inside(Path::new("/a/profiles-evil/x"), root));
        assert!(is_inside(Path::new("/a/profiles/./work/../home"), root));
        assert!(!is_inside(Path::new("/a/profiles/..x"), root));
    }

    #[cfg(windows)]
    #[test]
    fn is_inside_ignores_case_and_separators_on_windows() {
        let root = Path::new(r"C:\Users\Me\.multi-claude\profiles");
        assert!(is_inside(Path::new(r"c:/users/me/.multi-claude/profiles/work"), root));
        assert!(!is_inside(Path::new(r"D:\Users\Me\.multi-claude\profiles\work"), root));
    }
}
