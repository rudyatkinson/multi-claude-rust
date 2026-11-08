//! `a.localeCompare(b)` as the Electron version sorted folder and file names with it: the root
//! collation of ICU with its defaults (punctuation counts, case comes last). An approximation that
//! is exact for the names skills and their files have: white space, then punctuation and symbols
//! in ICU's order, then digits, then letters without regard to accents and case; on a tie the
//! letter without an accent comes first, then lower case before upper case.

use std::cmp::Ordering;

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// ASCII punctuation and symbols in the order of ICU's root collation.
const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// What a character weighs at the first level: a group and a place in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Primary(u8, u32);

/// One character of a name, split into what the three levels look at.
struct Unit {
    primary: Primary,
    /// The accents on it, by code point.
    accents: Vec<char>,
    upper: bool,
}

fn primary_of(base: char) -> Option<Primary> {
    if base.is_control() && !matches!(base, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r') {
        // ignorable
        return None;
    }
    if base.is_whitespace() {
        return Some(Primary(0, base as u32));
    }
    if let Some(at) = PUNCTUATION.find(base) {
        return Some(Primary(1, at as u32));
    }
    if base.is_ascii_digit() {
        return Some(Primary(3, base as u32 - '0' as u32));
    }
    if base.is_alphabetic() {
        let lower = base.to_lowercase().next().unwrap_or(base);
        return Some(Primary(4, lower as u32));
    }
    if base.is_numeric() {
        return Some(Primary(3, 10 + base as u32));
    }
    // other punctuation and symbols: after the ASCII ones, before the digits
    Some(Primary(2, base as u32))
}

fn units(s: &str) -> Vec<Unit> {
    let mut out: Vec<Unit> = Vec::new();
    for c in s.nfd() {
        if is_combining_mark(c) {
            if let Some(last) = out.last_mut() {
                last.accents.push(c);
            }
            continue;
        }
        if let Some(primary) = primary_of(c) {
            out.push(Unit { primary, accents: Vec::new(), upper: c.is_uppercase() });
        }
    }
    out
}

/// `a.localeCompare(b)` for names (see the module): the order the Electron version listed skills
/// and their files in.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let (ua, ub) = (units(a), units(b));
    ua.iter()
        .map(|u| u.primary)
        .cmp(ub.iter().map(|u| u.primary))
        .then_with(|| ua.iter().map(|u| &u.accents).cmp(ub.iter().map(|u| &u.accents)))
        .then_with(|| ua.iter().map(|u| u.upper).cmp(ub.iter().map(|u| u.upper)))
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
        out.sort_by(|a, b| locale_compare(a, b));
        out
    }

    #[test]
    fn letters_without_regard_to_case_then_lower_case_first() {
        assert_eq!(sorted(&["review", "Deploy", "b", "A", "a", "B"]), ["a", "A", "b", "B", "Deploy", "review"]);
    }

    #[test]
    fn punctuation_before_digits_before_letters() {
        assert_eq!(sorted(&["a1", "a-b", "a_b", "ab", "a"]), ["a", "a_b", "a-b", "a1", "ab"]);
        assert_eq!(sorted(&["z", "1", "-", " "]), [" ", "-", "1", "z"]);
    }

    #[test]
    fn accents_count_only_on_a_tie() {
        assert_eq!(sorted(&["éa", "eb", "ea"]), ["ea", "éa", "eb"]);
        assert_eq!(sorted(&["Ça", "cb"]), ["Ça", "cb"]);
    }

    #[test]
    fn longer_after_its_beginning() {
        assert_eq!(locale_compare("review", "review-fast"), Ordering::Less);
        assert_eq!(locale_compare("same", "same"), Ordering::Equal);
    }
}
