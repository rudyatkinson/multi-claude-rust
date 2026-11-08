//! Helpers that reproduce JavaScript string semantics. Every length limit in the Electron version
//! is a `string.length`, i.e. a count of UTF-16 code units, and `slice(0, n)` cuts at code units.
//! Ports use these instead of `str::len` / byte slicing so that limits match the TypeScript tests.

/// `s.length` in JavaScript: the number of UTF-16 code units.
pub fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(0, max)` in JavaScript, never splitting a surrogate pair: if the cut would fall inside
/// a pair, the whole character is dropped.
pub fn cut16(s: &str, max: usize) -> &str {
    let mut units = 0usize;
    for (idx, ch) in s.char_indices() {
        let w = ch.len_utf16();
        if units + w > max {
            return &s[..idx];
        }
        units += w;
    }
    s
}

/// `s.length > max` in JavaScript.
pub fn longer16(s: &str, max: usize) -> bool {
    let mut units = 0usize;
    for ch in s.chars() {
        units += ch.len_utf16();
        if units > max {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_utf16_units() {
        assert_eq!(len16("abc"), 3);
        assert_eq!(len16("Işık"), 4);
        assert_eq!(len16("😀"), 2);
    }

    #[test]
    fn cuts_at_units_without_splitting_pairs() {
        assert_eq!(cut16("abcdef", 3), "abc");
        assert_eq!(cut16("a😀b", 2), "a");
        assert_eq!(cut16("a😀b", 3), "a😀");
        assert_eq!(cut16("ab", 10), "ab");
    }

    #[test]
    fn longer_than() {
        assert!(longer16("😀😀", 3));
        assert!(!longer16("😀", 2));
    }
}
