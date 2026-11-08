//! String fields of records that were cut off: `decodeCut`, `fieldOfCut` and `wholeField` of
//! `src/main/conversations.ts`.

use std::sync::LazyLock;

use regex::Regex;

/// How much of a cut field is decoded, in UTF-16 units of its encoded text.
const CUT_UNITS: usize = 4000;

/// The `cwd` field of a record, only if its closing quote is there. `.` of the JavaScript pattern
/// stops at its line terminators, which the class here leaves out.
static WHOLE_CWD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""cwd":"((?:[^"\\]|\\[^\n\r\x{2028}\x{2029}])*)""#).unwrap());

/// Decodes the characters of a JSON string (what stands between its quotes) for as long as they
/// are valid, and at most `max_units` UTF-16 units of the encoded text. Returns the text and
/// whether all of `encoded` was taken.
///
/// `JSON.parse` keeps an escaped half of a surrogate pair as it is; a Rust string cannot hold one,
/// so it becomes U+FFFD.
fn decode_chars(encoded: &str, max_units: usize) -> (String, bool) {
    let mut out = String::new();
    let mut at = 0usize;
    let mut units = 0usize;
    while let Some(c) = encoded[at..].chars().next() {
        let rest = &encoded[at..];
        // what it stands for, and how many bytes (and, for an escape, units) it takes
        let (ch, taken) = match c {
            '"' | '\0'..='\u{1f}' => break,
            '\\' => match rest.as_bytes().get(1) {
                Some(b'"') => ('"', 2),
                Some(b'\\') => ('\\', 2),
                Some(b'/') => ('/', 2),
                Some(b'b') => ('\u{8}', 2),
                Some(b'f') => ('\u{c}', 2),
                Some(b'n') => ('\n', 2),
                Some(b'r') => ('\r', 2),
                Some(b't') => ('\t', 2),
                Some(b'u') => match hex4(&rest[2..]) {
                    Some(high @ 0xD800..=0xDBFF) => match rest.get(6..).and_then(low_escape) {
                        Some(low) => (pair(high, low), 12),
                        None => ('\u{FFFD}', 6),
                    },
                    Some(0xDC00..=0xDFFF) => ('\u{FFFD}', 6),
                    Some(unit) => (char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'), 6),
                    None => break,
                },
                _ => break,
            },
            _ => (c, c.len_utf8()),
        };
        let taken_units = if c == '\\' { taken } else { c.len_utf16() };
        if units + taken_units > max_units {
            break;
        }
        units += taken_units;
        at += taken;
        out.push(ch);
    }
    (out, at == encoded.len())
}

/// The four hex digits of a `\u` escape.
fn hex4(s: &str) -> Option<u16> {
    let digits = s.get(..4)?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(digits, 16).ok()
}

/// A `\uDC00`-`\uDFFF` escape at the start of `s`.
fn low_escape(s: &str) -> Option<u16> {
    let unit = hex4(s.strip_prefix("\\u")?)?;
    (0xDC00..=0xDFFF).contains(&unit).then_some(unit)
}

fn pair(high: u16, low: u16) -> char {
    let code = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

/// `decodeCut`: the start of a JSON string that was cut off somewhere, decoded as far as it goes
/// (the cut may be inside an escape). At most the first 4000 units of `encoded` are looked at.
fn decode_cut(encoded: &str) -> Option<String> {
    let (text, _) = decode_chars(encoded, CUT_UNITS);
    (!text.is_empty()).then_some(text)
}

/// `fieldOfCut`: the beginning of a string field in a record that was cut off, as far as it was read.
pub(super) fn field_of_cut(line: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let at = line.find(&marker)?;
    decode_cut(&line[at + marker.len()..])
}

/// `wholeField(text, 'cwd')`: the folder of a record, only if all of it is there: half a folder
/// path is another folder.
pub(super) fn whole_cwd(text: &str) -> Option<String> {
    let encoded = WHOLE_CWD.captures(text)?.get(1)?.as_str();
    let (cwd, whole) = decode_chars(encoded, usize::MAX);
    whole.then_some(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_as_far_as_the_text_is_valid() {
        assert_eq!(decode_cut(r#"lorem \"ipsum\" \\ dolor"#).as_deref(), Some(r#"lorem "ipsum" \ dolor"#));
        // cut inside an escape
        assert_eq!(decode_cut(r"ab\").as_deref(), Some("ab"));
        assert_eq!(decode_cut(r"ab\u00").as_deref(), Some("ab"));
        assert_eq!(decode_cut(r"ab\u00e7").as_deref(), Some("abç"));
        // the end of the field
        assert_eq!(decode_cut(r#"hello"},"cwd":"x"#).as_deref(), Some("hello"));
        assert_eq!(decode_cut(r#"","cwd":"x"#), None);
        assert_eq!(decode_cut(""), None);
        assert_eq!(decode_cut(r"\x"), None);
    }

    #[test]
    fn decodes_surrogate_pairs_and_stops_at_4000_units() {
        assert_eq!(decode_cut(r"\ud83d\ude00!").as_deref(), Some("😀!"));
        assert_eq!(decode_cut(r"\ud83d!").as_deref(), Some("\u{FFFD}!"));
        let long = "a".repeat(5000);
        assert_eq!(decode_cut(&long).map(|t| t.len()), Some(4000));
        let wide = "😀".repeat(2001);
        assert_eq!(decode_cut(&wide).map(|t| crate::jsstr::len16(&t)), Some(4000));
    }

    #[test]
    fn takes_a_folder_only_when_it_is_whole() {
        assert_eq!(whole_cwd(r#"{"cwd":"C:\\code\\app","x":1}"#).as_deref(), Some(r"C:\code\app"));
        assert_eq!(whole_cwd(r#"{"cwd":"C:\\code\\a"#), None);
        // an escape JSON does not know
        assert_eq!(whole_cwd(r#"{"cwd":"C:\code"}"#), None);
        assert_eq!(whole_cwd("{\"cwd\":\"a\nb\"}"), None);
    }
}
