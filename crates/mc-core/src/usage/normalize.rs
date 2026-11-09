//! The status line payload, and how usage samples combine (`src/main/usage/normalize.ts` and
//! `applyProbe` of `src/main/usage/index.ts`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::clock::days_from_civil;
use super::js_round;
use super::probe::ProbeResult;
use crate::types::{UsageState, UsageWindow};

/// Reset times this close together belong to the same window.
pub const SAME_WINDOW_MS: i64 = 90_000;
const OBSERVED_REFRESH_MS: i64 = 60_000;

/// The subscription rate limits a status line payload carries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSample {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

/// What a status line payload says about the session itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    /// Claude Code's id of the conversation (`session_id`).
    pub claude_session_id: Option<String>,
    pub transcript_path: Option<String>,
    /// `model.display_name`, e.g. "Opus".
    pub model: Option<String>,
    /// Whole percent of the context window in use, 0-100.
    pub context_pct: Option<u32>,
}

/// Everything a status file says, read with one parse: what [`parse_session_meta`] and
/// [`parse_status_payload`] return for the same text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusSample {
    pub meta: Option<SessionMeta>,
    pub usage: Option<UsageSample>,
}

/// Reads a status file. Text that is not JSON gives neither part.
pub fn parse_status(raw: &str) -> StatusSample {
    match serde_json::from_str::<Value>(raw) {
        Ok(json) => StatusSample { meta: session_meta(&json), usage: usage_sample(&json) },
        Err(_) => StatusSample::default(),
    }
}

/// Extracts the subscription rate limits from a status line stdin payload; None when it has none.
pub fn parse_status_payload(raw: &str) -> Option<UsageSample> {
    usage_sample(&serde_json::from_str(raw).ok()?)
}

/// What the payload says about its session; None unless it is a JSON object.
pub fn parse_session_meta(raw: &str) -> Option<SessionMeta> {
    session_meta(&serde_json::from_str(raw).ok()?)
}

fn usage_sample(json: &Value) -> Option<UsageSample> {
    let limits = json.get("rate_limits")?;
    let sample = UsageSample { five_hour: to_window(limits.get("five_hour")), seven_day: to_window(limits.get("seven_day")) };
    (sample.five_hour.is_some() || sample.seven_day.is_some()).then_some(sample)
}

fn to_window(raw: Option<&Value>) -> Option<UsageWindow> {
    let raw = raw?;
    let pct = raw.get("used_percentage")?.as_f64()?;
    let resets_at = match raw.get("resets_at")? {
        // documented as epoch seconds; tolerate milliseconds
        Value::Number(n) => {
            let resets = n.as_f64()?;
            (if resets < 1e12 { resets * 1000.0 } else { resets }) as i64
        }
        Value::String(text) => parse_iso_ms(text)?,
        _ => return None,
    };
    Some(UsageWindow { used_pct: pct.clamp(0.0, 100.0), resets_at: Some(resets_at) })
}

fn session_meta(json: &Value) -> Option<SessionMeta> {
    let payload = json.as_object()?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let pct = payload.get("context_window").and_then(|w| w.get("used_percentage")).and_then(Value::as_f64);
    Some(SessionMeta {
        claude_session_id: text(payload.get("session_id")),
        transcript_path: text(payload.get("transcript_path")),
        model: text(payload.get("model").and_then(|m| m.get("display_name"))),
        context_pct: pct.map(|p| js_round(p).clamp(0.0, 100.0) as u32),
    })
}

// `Date.parse` for the ISO 8601 forms: a date (UTC), or a date and time with `Z`, an offset, or
// neither. JavaScript reads a date and time without a zone as local time; this reads it as UTC.
static ISO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^([0-9]{4})(?:-([0-9]{2})(?:-([0-9]{2})(?:[Tt ]([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:[.,]([0-9]+))?)?\s*(Z|z|[+-][0-9]{2}:?[0-9]{2})?)?)?)?$",
    )
    .expect("valid regex")
});

/// Epoch ms of an ISO 8601 timestamp such as `2026-10-07T15:00:00Z`, `2026-10-07T18:00:00.250+03:00`
/// or `2026-10-07`; None for anything else.
pub fn parse_iso_ms(text: &str) -> Option<i64> {
    let caps = ISO.captures(text.trim())?;
    let num = |i: usize, default: i64| caps.get(i).map_or(Some(default), |m| m.as_str().parse::<i64>().ok());
    let (year, month, day) = (num(1, 0)?, num(2, 1)?, num(3, 1)?);
    let (hour, minute, second) = (num(4, 0)?, num(5, 0)?, num(6, 0)?);
    let millis = caps.get(7).map_or(0, |m| {
        let digits: String = m.as_str().chars().chain("000".chars()).take(3).collect();
        digits.parse::<i64>().unwrap_or(0)
    });
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (hour < 24 || (hour == 24 && minute == 0 && second == 0 && millis == 0))
        && minute < 60
        && second < 60;
    if !valid {
        return None;
    }
    let offset_minutes = match caps.get(8).map(|m| m.as_str()) {
        None | Some("Z") | Some("z") => 0,
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            let (h, m) = (digits[..2].parse::<i64>().ok()?, digits[2..].parse::<i64>().ok()?);
            if h > 23 || m > 59 {
                return None;
            }
            sign * (h * 60 + m)
        }
    };
    let days = days_from_civil(year, month as u32, day as u32);
    Some((((days * 24 + hour) * 60 + minute - offset_minutes) * 60 + second) * 1000 + millis)
}

/// The outcome of comparing one window of a sample with what is known.
struct Pick {
    win: Option<UsageWindow>,
    /// The sample's window was taken (the previous object is replaced).
    taken: bool,
    /// The sample is not older than what is known: it confirms or advances it.
    accepted: bool,
}

/// Several sessions of one account report independently, and an idle session keeps re-sending
/// the limits from its last response. Usage only grows inside a window, so a sample wins when it
/// belongs to a later window, or to the same window with a higher percentage.
fn pick(prev: Option<&UsageWindow>, next: Option<&UsageWindow>) -> Pick {
    let keep = |accepted: bool| Pick { win: prev.cloned(), taken: false, accepted };
    let take = || Pick { win: next.cloned(), taken: true, accepted: true };
    let Some(next) = next else { return keep(false) };
    let (Some(p), Some(prev_reset), Some(next_reset)) = (prev, prev.and_then(|p| p.resets_at), next.resets_at) else {
        return take();
    };
    if next_reset > prev_reset + SAME_WINDOW_MS {
        return take();
    }
    if next_reset < prev_reset - SAME_WINDOW_MS {
        return keep(false);
    }
    if next.used_pct > p.used_pct {
        return take();
    }
    keep(next.used_pct == p.used_pct)
}

/// Combines a status line sample with what is known of the account. None when the sample changes
/// nothing worth reporting (the Electron version returned the previous object then): it is older,
/// or it only confirms numbers that were confirmed less than a minute ago.
pub fn merge_usage(prev: Option<&UsageState>, sample: &UsageSample, now: i64) -> Option<UsageState> {
    let five = pick(prev.and_then(|p| p.five_hour.as_ref()), sample.five_hour.as_ref());
    let seven = pick(prev.and_then(|p| p.seven_day.as_ref()), sample.seven_day.as_ref());
    let Some(prev) = prev else {
        return Some(UsageState { five_hour: five.win, seven_day: seven.win, models: None, observed_at: Some(now) });
    };
    let changed = five.taken || seven.taken;
    let accepted = five.accepted || seven.accepted;
    if !changed && (!accepted || now - prev.observed_at.unwrap_or(0) < OBSERVED_REFRESH_MS) {
        return None;
    }
    // the status line knows nothing about per-model limits, so those are carried over
    Some(UsageState {
        five_hour: five.win,
        seven_day: seven.win,
        models: prev.models.clone(),
        observed_at: if accepted { Some(now) } else { prev.observed_at },
    })
}

/// `/usage` is the server's current answer, so it replaces what was known; a window it did not
/// report stays as it was, and its model limits replace the old ones.
pub fn apply_probe(prev: Option<&UsageState>, result: &ProbeResult, now: i64) -> UsageState {
    // `/usage` lists every window the plan has. Weekly lines without a session line mean the plan
    // has no 5-hour limit (Ultra): a window kept from before the plan changed goes.
    let has_weekly = result.seven_day.is_some() || !result.models.is_empty();
    let five_hour = match &result.five_hour {
        Some(window) => Some(window.clone()),
        None if has_weekly => None,
        None => prev.and_then(|p| p.five_hour.clone()),
    };
    UsageState {
        five_hour,
        seven_day: result.seven_day.clone().or_else(|| prev.and_then(|p| p.seven_day.clone())),
        models: Some(result.models.clone()),
        observed_at: Some(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelUsage;

    const HOUR: i64 = 3_600_000;

    fn win(used_pct: f64, resets_at: i64) -> Option<UsageWindow> {
        Some(UsageWindow { used_pct, resets_at: Some(resets_at) })
    }

    fn five(used_pct: f64, resets_at: i64) -> UsageSample {
        UsageSample { five_hour: win(used_pct, resets_at), seven_day: None }
    }

    // ---------- parseStatusPayload ----------

    #[test]
    fn reads_the_documented_status_line_shape() {
        let payload = r#"{"model":{"display_name":"Opus"},"rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},"seven_day":{"used_percentage":41.2,"resets_at":1738857600}}}"#;
        assert_eq!(
            parse_status_payload(payload),
            Some(UsageSample { five_hour: win(23.5, 1_738_425_600_000), seven_day: win(41.2, 1_738_857_600_000) })
        );
    }

    #[test]
    fn accepts_a_single_window_and_iso_timestamps() {
        let payload = r#"{"rate_limits":{"five_hour":{"used_percentage":140,"resets_at":"2026-10-07T15:00:00Z"}}}"#;
        assert_eq!(parse_status_payload(payload), Some(UsageSample { five_hour: win(100.0, 1_791_385_200_000), seven_day: None }));
    }

    #[test]
    fn tolerates_milliseconds() {
        let payload = r#"{"rate_limits":{"seven_day":{"used_percentage":-3,"resets_at":1738857600000}}}"#;
        assert_eq!(parse_status_payload(payload), Some(UsageSample { five_hour: None, seven_day: win(0.0, 1_738_857_600_000) }));
    }

    #[test]
    fn returns_none_without_usable_rate_limits() {
        assert_eq!(parse_status_payload(r#"{"model":{}}"#), None);
        assert_eq!(parse_status_payload(r#"{"rate_limits":{}}"#), None);
        assert_eq!(parse_status_payload(r#"{"rate_limits":{"five_hour":{"used_percentage":null}}}"#), None);
        assert_eq!(parse_status_payload(r#"{"rate_lim"#), None);
        // a window without a reset time is not used
        assert_eq!(parse_status_payload(r#"{"rate_limits":{"five_hour":{"used_percentage":5}}}"#), None);
        assert_eq!(parse_status_payload(r#"{"rate_limits":{"five_hour":{"used_percentage":5,"resets_at":"soon"}}}"#), None);
        assert_eq!(parse_status_payload("null"), None);
    }

    #[test]
    fn reads_what_the_payload_says_about_the_session() {
        let payload = r#"{"session_id":"abc","transcript_path":"","model":{"display_name":"Fable 5"},"context_window":{"used_percentage":41.5}}"#;
        assert_eq!(
            parse_session_meta(payload),
            Some(SessionMeta {
                claude_session_id: Some("abc".into()),
                transcript_path: None,
                model: Some("Fable 5".into()),
                context_pct: Some(42),
            })
        );
        assert_eq!(parse_session_meta(r#"{"context_window":{"used_percentage":180}}"#).unwrap().context_pct, Some(100));
        assert_eq!(parse_session_meta(r#"{"model":"Opus"}"#), Some(SessionMeta::default()));
        assert_eq!(parse_session_meta("[1]"), None);
        assert_eq!(parse_session_meta("{"), None);
    }

    #[test]
    fn reads_both_parts_with_one_parse() {
        let status = parse_status(r#"{"session_id":"s","rate_limits":{"five_hour":{"used_percentage":7,"resets_at":1738425600}}}"#);
        assert_eq!(status.meta.and_then(|m| m.claude_session_id), Some("s".into()));
        assert_eq!(status.usage, Some(five(7.0, 1_738_425_600_000)));
        assert_eq!(parse_status("nope"), StatusSample::default());
    }

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(parse_iso_ms("2026-10-07T15:00:00Z"), Some(1_791_385_200_000));
        assert_eq!(parse_iso_ms("2026-10-07T18:00:00+03:00"), Some(1_791_385_200_000));
        assert_eq!(parse_iso_ms("2026-10-07T10:30:00.250-0430"), Some(1_791_385_200_250));
        assert_eq!(parse_iso_ms("2026-10-07T15:00Z"), Some(1_791_385_200_000));
        assert_eq!(parse_iso_ms("2026-10-07"), Some(1_791_331_200_000));
        assert_eq!(parse_iso_ms("2026-10-08T00:00:00.123456Z"), Some(1_791_417_600_123));
        assert_eq!(parse_iso_ms("2026-10-07T24:00:00Z"), Some(1_791_417_600_000));
        assert_eq!(parse_iso_ms("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso_ms("2026-10-07T15:60:00Z"), None);
        assert_eq!(parse_iso_ms("Oct 7"), None);
        assert_eq!(parse_iso_ms(""), None);
    }

    // ---------- mergeUsage ----------

    const NOW: i64 = 1_000_000_000_000;
    const RESET: i64 = NOW + 2 * HOUR;

    fn prev() -> UsageState {
        UsageState { five_hour: win(40.0, RESET), seven_day: None, models: None, observed_at: Some(NOW) }
    }

    #[test]
    fn takes_a_higher_percentage_in_the_same_window() {
        let next = merge_usage(Some(&prev()), &five(45.0, RESET + 1000), NOW + 5000).unwrap();
        assert_eq!(next.five_hour.unwrap().used_pct, 45.0);
        assert_eq!(next.observed_at, Some(NOW + 5000));
    }

    #[test]
    fn ignores_an_idle_session_re_sending_older_numbers() {
        assert_eq!(merge_usage(Some(&prev()), &five(12.0, RESET), NOW + 5000), None);
        assert_eq!(merge_usage(Some(&prev()), &five(90.0, RESET - 5 * HOUR), NOW + 5000), None);
    }

    #[test]
    fn moves_to_a_later_window_even_when_its_percentage_is_lower() {
        let next = merge_usage(Some(&prev()), &five(3.0, RESET + 5 * HOUR), NOW + 5000).unwrap();
        assert_eq!(next.five_hour, win(3.0, RESET + 5 * HOUR));
    }

    #[test]
    fn refreshes_the_observation_time_for_a_confirming_sample_only_after_a_while() {
        let same = five(40.0, RESET);
        assert_eq!(merge_usage(Some(&prev()), &same, NOW + 5000), None);
        assert_eq!(merge_usage(Some(&prev()), &same, NOW + 120_000).unwrap().observed_at, Some(NOW + 120_000));
    }

    #[test]
    fn keeps_a_window_the_sample_no_longer_carries() {
        let both = UsageState { seven_day: win(10.0, NOW + 100 * HOUR), ..prev() };
        let next = merge_usage(Some(&both), &five(50.0, RESET), NOW + 1).unwrap();
        assert_eq!(next.seven_day, both.seven_day);
    }

    #[test]
    fn starts_from_the_first_sample() {
        let next = merge_usage(None, &five(5.0, RESET), NOW).unwrap();
        assert_eq!(next, UsageState { five_hour: win(5.0, RESET), seven_day: None, models: None, observed_at: Some(NOW) });
    }

    // ---------- status line samples after a probe (test/probe.test.ts) ----------

    #[test]
    fn keep_the_model_limits_the_status_line_does_not_know_about() {
        let reset = NOW + HOUR;
        let models = Some(vec![ModelUsage { name: "Fable".into(), used_pct: 35.0, resets_at: Some(reset) }]);
        let probed = UsageState { five_hour: win(40.0, reset), seven_day: None, models: models.clone(), observed_at: Some(NOW) };
        let next = merge_usage(Some(&probed), &five(45.0, reset), NOW + 1000).unwrap();
        assert_eq!(next.five_hour.unwrap().used_pct, 45.0);
        assert_eq!(next.models, models);
    }

    #[test]
    fn replace_a_window_whose_reset_time_was_unknown() {
        let reset = NOW + HOUR;
        let unknown = UsageState {
            five_hour: Some(UsageWindow { used_pct: 0.0, resets_at: None }),
            seven_day: None,
            models: None,
            observed_at: Some(NOW),
        };
        assert_eq!(merge_usage(Some(&unknown), &five(3.0, reset), NOW + 1000).unwrap().five_hour, win(3.0, reset));
    }

    // ---------- applyProbe ----------

    #[test]
    fn a_probe_replaces_what_it_reports_and_keeps_the_rest() {
        let known = UsageState { five_hour: win(40.0, RESET), seven_day: win(10.0, RESET + 90 * HOUR), models: None, observed_at: Some(NOW) };
        let result = ProbeResult { five_hour: win(20.0, RESET + 5 * HOUR), seven_day: None, models: vec![] };
        let next = apply_probe(Some(&known), &result, NOW + 10);
        assert_eq!(
            next,
            UsageState { five_hour: win(20.0, RESET + 5 * HOUR), seven_day: known.seven_day.clone(), models: Some(vec![]), observed_at: Some(NOW + 10) }
        );
        assert_eq!(apply_probe(None, &result, NOW).seven_day, None);
    }

    #[test]
    fn a_probe_with_weekly_lines_but_no_session_line_drops_the_5_hour_window() {
        // an Ultra plan: `/usage` prints the weekly limits only
        let known = UsageState { five_hour: win(12.0, RESET), seven_day: win(10.0, RESET + 90 * HOUR), models: None, observed_at: Some(NOW) };
        let result = ProbeResult { five_hour: None, seven_day: win(23.0, RESET + 90 * HOUR), models: vec![] };
        let next = apply_probe(Some(&known), &result, NOW + 10);
        assert_eq!(next.five_hour, None);
        assert_eq!(next.seven_day, win(23.0, RESET + 90 * HOUR));
        // a probe that reports nothing keeps what was known
        let empty = ProbeResult { five_hour: None, seven_day: None, models: vec![] };
        assert_eq!(apply_probe(Some(&known), &empty, NOW + 10).five_hour, known.five_hour);
    }
}
