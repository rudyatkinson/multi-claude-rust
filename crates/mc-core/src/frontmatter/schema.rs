//! The YAML 1.2 core schema as the `yaml` npm package applies it: which plain scalars are null,
//! booleans or numbers, and how JavaScript turns those values back into text. Both the reader
//! (what a field says) and the writer (when a text must stand in quotes) go by it.

use std::sync::LazyLock;

use regex::Regex;

/// A scalar as JavaScript holds it after `yaml` read it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Resolved {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Resolved {
    /// `String(value)` in JavaScript.
    pub(super) fn js_string(&self) -> String {
        match self {
            Resolved::Null => "null".to_owned(),
            Resolved::Bool(b) => b.to_string(),
            Resolved::Num(n) => js_number_string(*n),
            Resolved::Str(s) => s.clone(),
        }
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static NULL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:~|[Nn]ull|NULL)?$"));
static BOOL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[Tt]rue|TRUE|[Ff]alse|FALSE)$"));
static INT_OCT: LazyLock<Regex> = LazyLock::new(|| re(r"^0o[0-7]+$"));
static INT: LazyLock<Regex> = LazyLock::new(|| re(r"^[-+]?[0-9]+$"));
static INT_HEX: LazyLock<Regex> = LazyLock::new(|| re(r"^0x[0-9a-fA-F]+$"));
static FLOAT_NAN: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[-+]?\.(?:inf|Inf|INF)|\.nan|\.NaN|\.NAN)$"));
static FLOAT_EXP: LazyLock<Regex> = LazyLock::new(|| re(r"^[-+]?(?:\.[0-9]+|[0-9]+(?:\.[0-9]*)?)[eE][-+]?[0-9]+$"));
static FLOAT: LazyLock<Regex> = LazyLock::new(|| re(r"^[-+]?(?:\.[0-9]+|[0-9]+\.[0-9]*)$"));

/// A plain text that the core schema reads as something other than a string: such a text has to
/// stand in quotes to stay a text.
pub(super) fn reads_as_non_string(plain: &str) -> bool {
    [&NULL, &BOOL, &INT_OCT, &INT, &INT_HEX, &FLOAT_NAN, &FLOAT_EXP, &FLOAT].iter().any(|r| r.is_match(plain))
}

fn radix_value(digits: &str, radix: u32) -> f64 {
    digits.chars().fold(0.0, |acc, c| acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0)))
}

/// What a plain scalar without a tag is.
pub(super) fn resolve_plain(value: &str) -> Resolved {
    if NULL.is_match(value) {
        return Resolved::Null;
    }
    if BOOL.is_match(value) {
        return Resolved::Bool(value.eq_ignore_ascii_case("true"));
    }
    if INT_OCT.is_match(value) {
        return Resolved::Num(radix_value(&value[2..], 8));
    }
    if INT.is_match(value) {
        return Resolved::Num(value.parse::<f64>().unwrap_or(f64::NAN));
    }
    if INT_HEX.is_match(value) {
        return Resolved::Num(radix_value(&value[2..], 16));
    }
    if FLOAT_NAN.is_match(value) {
        return Resolved::Num(if value.ends_with("nan") || value.ends_with("NaN") || value.ends_with("NAN") {
            f64::NAN
        } else if value.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    if FLOAT_EXP.is_match(value) || FLOAT.is_match(value) {
        return Resolved::Num(value.parse::<f64>().unwrap_or(f64::NAN));
    }
    Resolved::Str(value.to_owned())
}

/// A scalar with an explicit tag. `!!str`, `!!null`, `!!bool`, `!!int` and `!!float` are the core
/// schema's own; any other tag reads as a string, as `yaml` falls back to one (with a warning, not
/// an error).
pub(super) fn resolve_tagged(value: &str, handle: &str, suffix: &str) -> Resolved {
    if handle == "tag:yaml.org,2002:" {
        match suffix {
            "null" => return Resolved::Null,
            "bool" if BOOL.is_match(value) => return Resolved::Bool(value.eq_ignore_ascii_case("true")),
            "int" | "float" => {
                if let Resolved::Num(n) = resolve_plain(value) {
                    return Resolved::Num(n);
                }
            }
            _ => {}
        }
    }
    Resolved::Str(value.to_owned())
}

/// `String(n)` for a JavaScript number.
pub(super) fn js_number_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_owned();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-Infinity".to_owned() } else { "Infinity".to_owned() };
    }
    if n == 0.0 {
        return "0".to_owned();
    }
    let abs = n.abs();
    if !(1e-6..1e21).contains(&abs) {
        // JavaScript writes the exponent with its sign: 1e+21, 1.5e-7
        let s = format!("{n:e}");
        return match s.split_once('e') {
            Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
            _ => s,
        };
    }
    format!("{n}")
}

/// `Number(text)` in JavaScript: NaN for anything that is not a number as a whole.
pub(super) fn js_number_parse(text: &str) -> f64 {
    static DECIMAL: LazyLock<Regex> = LazyLock::new(|| re(r"^[-+]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?$"));
    let s = super::js_trim(text);
    if s.is_empty() {
        return 0.0;
    }
    match s {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = s.strip_prefix(prefix) {
            return if !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix)) { radix_value(digits, radix) } else { f64::NAN };
        }
    }
    if DECIMAL.is_match(s) {
        s.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}
