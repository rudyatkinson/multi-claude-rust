//! `sanitize_settings`, `merge_settings`, `stable_stringify` and the fingerprints of the files an
//! account copies from the main account (`src/main/sharing.ts`).
//!
//! JSON text is produced as `JSON.stringify` writes it (numbers like `1`, not `1.0`; `1e+21`;
//! the same escapes), so that the fingerprints the Electron version stored in `lastSync` still
//! match after the move.

use std::cmp::Ordering;
use std::fmt::Write as _;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::env::AUTH_SCRUB;

/// The settings key that runs a script for an API key, which would replace the account's login.
pub const API_KEY_HELPER: &str = "apiKeyHelper";

/// True for a variable of [`AUTH_SCRUB`], in any letter case (`toUpperCase`).
fn is_auth_var(name: &str) -> bool {
    AUTH_SCRUB.contains(&name.to_uppercase().as_str())
}

// ---------- JSON text as JavaScript writes it ----------

/// `String(n)` / `JSON.stringify(n)` of a finite number.
pub fn js_number(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        // JSON has no NaN or infinity; JSON.stringify writes them as null
        return if n.is_finite() { "0".to_owned() } else { "null".to_owned() };
    }
    // the shortest digits that read back as `n`, as `d.ddde<exp>`
    let sci = format!("{:e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').expect("exponent form");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exp: i32 = exp.parse().expect("integer exponent");
    let k = digits.len() as i32;
    // the decimal point goes after `n` digits: value = 0.digits * 10^n
    let point = exp + 1;
    let mut out = String::new();
    if n < 0.0 {
        out.push('-');
    }
    if k <= point && point <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (point - k) as usize));
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let e = point - 1;
        let _ = write!(out, "e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    out
}

/// `JSON.stringify(s)` of a string.
fn push_js_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_scalar(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&js_number(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => push_js_string(out, s),
        Value::Array(_) | Value::Object(_) => unreachable!("not a scalar"),
    }
}

/// Keys in the order `Array.prototype.sort` puts them: by UTF-16 code units.
fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn push_stable(out: &mut String, value: &Value) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_stable(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| cmp_utf16(a, b));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_js_string(out, key);
                out.push(':');
                push_stable(out, &map[key]);
            }
            out.push('}');
        }
        scalar => push_scalar(out, scalar),
    }
}

/// JSON with sorted keys and no spaces: equal data gives equal text.
pub fn stable_stringify(value: &Value) -> String {
    let mut out = String::new();
    push_stable(&mut out, value);
    out
}

fn push_js(out: &mut String, value: &Value, indent: Option<&str>, depth: usize) {
    let (open, close, items): (char, char, Vec<(Option<&String>, &Value)>) = match value {
        Value::Array(items) => ('[', ']', items.iter().map(|v| (None, v)).collect()),
        Value::Object(map) => ('{', '}', map.iter().map(|(k, v)| (Some(k), v)).collect()),
        scalar => return push_scalar(out, scalar),
    };
    out.push(open);
    if items.is_empty() {
        out.push(close);
        return;
    }
    for (i, (key, item)) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if let Some(step) = indent {
            out.push('\n');
            out.push_str(&step.repeat(depth + 1));
        }
        if let Some(key) = key {
            push_js_string(out, key);
            out.push(':');
            if indent.is_some() {
                out.push(' ');
            }
        }
        push_js(out, item, indent, depth + 1);
    }
    if let Some(step) = indent {
        out.push('\n');
        out.push_str(&step.repeat(depth));
    }
    out.push(close);
}

/// `JSON.stringify(value)`, keys in the order of the map.
pub fn to_js_json(value: &Value) -> String {
    let mut out = String::new();
    push_js(&mut out, value, None, 0);
    out
}

/// `JSON.stringify(value, null, 2)`, keys in the order of the map.
pub fn to_js_json_pretty(value: &Value) -> String {
    let mut out = String::new();
    push_js(&mut out, value, Some("  "), 0);
    out
}

// ---------- settings ----------

/// Drops settings that would override the account's own login: `apiKeyHelper`, and the auth
/// variables of `env` (an `env` left empty goes as well). Anything but an object is returned as it is.
pub fn sanitize_settings(settings: &Value) -> Value {
    let Some(obj) = settings.as_object() else {
        return settings.clone();
    };
    Value::Object(sanitized(obj))
}

fn sanitized(settings: &Map<String, Value>) -> Map<String, Value> {
    let mut out = settings.clone();
    out.remove(API_KEY_HELPER);
    if let Some(Value::Object(env)) = out.get("env") {
        let kept: Map<String, Value> = env.iter().filter(|(k, _)| !is_auth_var(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        if kept.is_empty() {
            out.remove("env");
        } else {
            out.insert("env".to_owned(), Value::Object(kept));
        }
    }
    out
}

/// Main's settings with the account's own values for `local_keys`. An account without a settings
/// file yet (`local` None) starts from Main's values for those keys too.
pub fn merge_settings(main: &Value, local: Option<&Value>, local_keys: &[String]) -> Value {
    let mut out = match main.as_object() {
        Some(obj) => sanitized(obj),
        None => Map::new(),
    };
    let Some(local) = local.and_then(Value::as_object) else {
        return Value::Object(out);
    };
    for key in local_keys {
        match local.get(key) {
            Some(value) => {
                out.insert(key.clone(), value.clone());
            }
            None => {
                out.remove(key);
            }
        }
    }
    Value::Object(out)
}

/// sha256 hex.
pub fn fingerprint_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The hash of the part of a settings file that sync owns, so that changes to the account's own
/// keys do not count as drift.
pub fn fingerprint_settings(settings: &Value, local_keys: &[String]) -> String {
    let mut rest = match sanitize_settings(settings) {
        Value::Object(obj) => obj,
        other => return fingerprint_bytes(stable_stringify(&other).as_bytes()),
    };
    for key in local_keys {
        rest.remove(key);
    }
    fingerprint_bytes(stable_stringify(&Value::Object(rest)).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn strips_settings_that_would_override_the_profile_login() {
        let out = sanitize_settings(&json!({
            "model": "opus",
            "apiKeyHelper": "get-key.sh",
            "env": { "ANTHROPIC_API_KEY": "x", "claude_code_oauth_token": "y", "FOO": "bar" }
        }));
        assert_eq!(out, json!({ "model": "opus", "env": { "FOO": "bar" } }));
    }

    #[test]
    fn drops_an_env_block_that_only_held_auth_variables() {
        assert_eq!(sanitize_settings(&json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "x" } })), json!({}));
        // an env that is not an object is left alone
        assert_eq!(sanitize_settings(&json!({ "env": [1] })), json!({ "env": [1] }));
    }

    #[test]
    fn starts_a_new_profile_from_all_of_main_then_keeps_its_own_local_keys() {
        let main = json!({ "model": "opus", "effortLevel": "high", "theme": "dark" });
        let local_keys = keys(&["model", "effortLevel"]);
        assert_eq!(merge_settings(&main, None, &local_keys), main);
        assert_eq!(
            merge_settings(&main, Some(&json!({ "model": "sonnet", "theme": "light" })), &local_keys),
            json!({ "model": "sonnet", "theme": "dark" })
        );
    }

    #[test]
    fn stable_stringify_sorts_keys_at_every_level() {
        let a = json!({ "b": 1, "a": { "y": [1, { "d": 2, "c": 3 }], "x": null } });
        assert_eq!(stable_stringify(&a), r#"{"a":{"x":null,"y":[1,{"c":3,"d":2}]},"b":1}"#);
        // by UTF-16 units: U+FF5E sorts after U+1F600's surrogates start (0xD83D)
        let b = json!({ "\u{ff5e}": 1, "😀": 2 });
        assert_eq!(stable_stringify(&b), "{\"😀\":2,\"\u{ff5e}\":1}");
    }

    #[test]
    fn numbers_and_strings_are_written_as_javascript_writes_them() {
        let cases = [
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (100.0, "100"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.25e-7, "1.25e-7"),
            (123.456, "123.456"),
            (0.0, "0"),
            (-0.0, "0"),
            (9007199254740993.0, "9007199254740992"),
        ];
        for (n, text) in cases {
            assert_eq!(js_number(n), text, "{n}");
        }
        let v: Value = serde_json::from_str(r#"{"f":1.0,"s":"a\"b\\c\n\u0001\u007f/é"}"#).unwrap();
        assert_eq!(stable_stringify(&v), "{\"f\":1,\"s\":\"a\\\"b\\\\c\\n\\u0001\u{7f}/é\"}");
    }

    #[test]
    fn pretty_text_is_json_stringify_with_two_spaces() {
        let v = json!({ "a": [], "b": {}, "c": [1, { "d": true }] });
        assert_eq!(to_js_json_pretty(&v), "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": true\n    }\n  ]\n}");
        assert_eq!(to_js_json(&v), r#"{"a":[],"b":{},"c":[1,{"d":true}]}"#);
        assert_eq!(to_js_json(&json!("x")), r#""x""#);
    }

    #[test]
    fn fingerprints_ignore_local_keys_order_and_auth_settings() {
        let local_keys = keys(&["model"]);
        let a = fingerprint_settings(&json!({ "model": "opus", "theme": "dark" }), &local_keys);
        let b = fingerprint_settings(&json!({ "theme": "dark", "model": "sonnet", "apiKeyHelper": "x" }), &local_keys);
        assert_eq!(a, b);
        assert_ne!(a, fingerprint_settings(&json!({ "theme": "light" }), &local_keys));
        // sha256 of the stable text
        assert_eq!(a, fingerprint_bytes(br#"{"theme":"dark"}"#));
        assert_eq!(fingerprint_bytes(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
