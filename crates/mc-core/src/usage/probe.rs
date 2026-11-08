//! Claude Code's own non-interactive `/usage` (`src/main/usage/probe.ts`, the parsing half; the
//! queue that runs it lives in the backend crate).

use std::sync::LazyLock;

use regex::Regex;

use super::clock::{CivilDateTime, LocalClock};
use super::js_trim;
use crate::types::{ModelUsage, UsageWindow};

/// No session file, and none of the user's hooks, plugins or MCP servers: only the built-in
/// command runs. It makes no model call.
pub const PROBE_ARGS: [&str; 6] = ["-p", "/usage", "--no-session-persistence", "--setting-sources", "project", "--strict-mcp-config"];

/// What `claude -p "/usage"` reports for an account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProbeResult {
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
    /// Weekly limits that apply to a single model, e.g. "Fable", in the order printed.
    pub models: Vec<ModelUsage>,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

// `/^(?:([A-Za-z]{3})[a-z]*\.?\s+(\d{1,2})(?:,\s*(\d{4}))?,?\s+)?(\d{1,2})(?::(\d{2}))?\s*([ap])m\b/i`,
// with the case folding, digits and word boundary kept to ASCII as JavaScript has them
static RESET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:([A-Za-z]{3})[A-Za-z]*\.?\s+([0-9]{1,2})(?:,\s*([0-9]{4}))?,?\s+)?([0-9]{1,2})(?::([0-9]{2}))?\s*([apAP])[mM](?-u:\b)",
    )
    .expect("valid regex")
});

// e.g. "Current week (Fable): 35% used · resets Oct 11, 12am (Europe/Istanbul)"; the `R` flag makes
// `^`/`$` see `\r` as a line end and keeps `.` off it, as JavaScript's multiline mode does
static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?imR)^\s*Current (session|week \(([^)]+)\)):\s*([0-9]+(?:\.[0-9]+)?)% used(?:\s*\S{0,3}\s*resets\s+(.+?))?\s*$")
        .expect("valid regex")
});

static ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+only$").expect("valid regex"));

/// Turns the reset time Claude Code prints ("Oct 7, 7:10pm (Europe/Istanbul)", "Oct 11, 12am",
/// or just "7:10pm") into epoch ms. The text is in the machine's own time zone and usually has no
/// year, so the year is the one that puts the reset closest to `now`. A bare time is today, or
/// tomorrow once it is more than a minute past.
pub fn parse_reset_time(text: &str, now: i64, clock: &dyn LocalClock) -> Option<i64> {
    let caps = RESET.captures(js_trim(text))?;
    let number = |i: usize| caps.get(i).and_then(|m| m.as_str().parse::<i64>().ok());
    let pm = caps.get(6).is_some_and(|m| m.as_str().eq_ignore_ascii_case("p"));
    let hour = number(4)? % 12 + if pm { 12 } else { 0 };
    let minute = number(5).unwrap_or(0);
    let today = clock.local_at(now);

    let Some(month_name) = caps.get(1) else {
        // time only: today, or tomorrow once that time has passed
        let at = CivilDateTime::normalized(i64::from(today.year), i64::from(today.month) - 1, i64::from(today.day), hour, minute);
        let ms = clock.to_epoch_ms(at);
        return Some(if ms < now - 60_000 { clock.to_epoch_ms(at.plus_days(1)) } else { ms });
    };
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month_name.as_str()))? as i64;
    let day = number(2)?;
    let at = |year: i64| clock.to_epoch_ms(CivilDateTime::normalized(year, month, day, hour, minute));
    if let Some(year) = number(3) {
        return Some(at(year));
    }
    let year = i64::from(today.year);
    // the earliest of equally close candidates wins, as the reduce over [-1, 0, 1] picked it
    [year - 1, year, year + 1].into_iter().map(at).reduce(|best, ms| if (ms - now).abs() < (best - now).abs() { ms } else { best })
}

/// Reads the plan usage lines out of the `/usage` text; None when there are none.
pub fn parse_usage_output(text: &str, now: i64, clock: &dyn LocalClock) -> Option<ProbeResult> {
    let mut result = ProbeResult::default();
    let mut found = false;
    for caps in LINE.captures_iter(text) {
        found = true;
        let used_pct = caps[3].parse::<f64>().unwrap_or(0.0).clamp(0.0, 100.0);
        let resets_at = caps.get(4).and_then(|m| parse_reset_time(m.as_str(), now, clock));
        let window = UsageWindow { used_pct, resets_at };
        match caps.get(2).map(|m| js_trim(m.as_str())).filter(|scope| !scope.is_empty()) {
            None => result.five_hour = Some(window),
            Some(scope) if scope.eq_ignore_ascii_case("all models") => result.seven_day = Some(window),
            Some(scope) => result.models.push(ModelUsage {
                name: ONLY.replace(scope, "").into_owned(),
                used_pct: window.used_pct,
                resets_at: window.resets_at,
            }),
        }
    }
    found.then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::clock::FixedOffsetClock;

    // real `claude -p "/usage"` output of three accounts
    const BILAL: &str = "You are currently using your subscription to power your Claude Code usage

Current session: 4% used · resets Oct 7, 7:10pm (Europe/Istanbul)
Current week (all models): 60% used · resets Oct 11, 12am (Europe/Istanbul)
Current week (Fable): 35% used · resets Oct 11, 12am (Europe/Istanbul)

What's contributing to your limits usage?
Approximate, based on local sessions on this machine — does not include other devices or claude.ai.

Last 24h · 4592 requests · 9 sessions
  94% of your usage came from subagent-heavy sessions
";
    const ELIF: &str = "Current session: 2% used · resets Oct 7, 3:50pm (Europe/Istanbul)
Current week (all models): 0% used · resets Oct 13, 1pm (Europe/Istanbul)
Current week (Fable): 0% used · resets Oct 13, 1pm (Europe/Istanbul)
";
    const EMIR: &str = "Current session: 56% used · resets Oct 7, 5:49pm (Europe/Istanbul)\r
Current week (all models): 92% used · resets Oct 9, 7:59am (Europe/Istanbul)\r
Current week (Fable): 2% used · resets Oct 9, 7:59am (Europe/Istanbul)\r
";

    /// The TypeScript tests build their expectations in the machine's local time; here every case
    /// runs in several fixed zones (none of the dates involved is near a daylight saving change).
    const ZONES: [FixedOffsetClock; 4] = [
        FixedOffsetClock::UTC,
        FixedOffsetClock { offset_minutes: 180 },
        FixedOffsetClock { offset_minutes: -300 },
        FixedOffsetClock { offset_minutes: 345 },
    ];

    fn local(clock: &dyn LocalClock, year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        clock.to_epoch_ms(CivilDateTime::normalized(year, month - 1, day, hour, minute))
    }

    fn window(used_pct: f64, resets_at: i64) -> Option<UsageWindow> {
        Some(UsageWindow { used_pct, resets_at: Some(resets_at) })
    }

    fn model(name: &str, used_pct: f64, resets_at: i64) -> ModelUsage {
        ModelUsage { name: name.to_string(), used_pct, resets_at: Some(resets_at) }
    }

    #[test]
    fn reads_session_week_and_the_model_specific_week() {
        for clock in &ZONES {
            let now = local(clock, 2026, 10, 7, 15, 20);
            assert_eq!(
                parse_usage_output(BILAL, now, clock),
                Some(ProbeResult {
                    five_hour: window(4.0, local(clock, 2026, 10, 7, 19, 10)),
                    seven_day: window(60.0, local(clock, 2026, 10, 11, 0, 0)),
                    models: vec![model("Fable", 35.0, local(clock, 2026, 10, 11, 0, 0))],
                })
            );
        }
    }

    #[test]
    fn handles_zero_usage_afternoon_hours_and_windows_line_endings() {
        for clock in &ZONES {
            let now = local(clock, 2026, 10, 7, 15, 20);
            assert_eq!(
                parse_usage_output(ELIF, now, clock),
                Some(ProbeResult {
                    five_hour: window(2.0, local(clock, 2026, 10, 7, 15, 50)),
                    seven_day: window(0.0, local(clock, 2026, 10, 13, 13, 0)),
                    models: vec![model("Fable", 0.0, local(clock, 2026, 10, 13, 13, 0))],
                })
            );
            assert_eq!(
                parse_usage_output(EMIR, now, clock),
                Some(ProbeResult {
                    five_hour: window(56.0, local(clock, 2026, 10, 7, 17, 49)),
                    seven_day: window(92.0, local(clock, 2026, 10, 9, 7, 59)),
                    models: vec![model("Fable", 2.0, local(clock, 2026, 10, 9, 7, 59))],
                })
            );
        }
    }

    #[test]
    fn names_a_sonnet_only_limit_after_the_model_and_keeps_several_model_limits() {
        let clock = &FixedOffsetClock::UTC;
        let now = local(clock, 2026, 10, 7, 15, 20);
        let out = "Current week (Sonnet only): 12.5% used · resets Oct 11, 12am (UTC)\nCurrent week (Fable): 3% used · resets Oct 11, 12am (UTC)";
        let models: Vec<(String, f64)> =
            parse_usage_output(out, now, clock).unwrap().models.into_iter().map(|m| (m.name, m.used_pct)).collect();
        assert_eq!(models, vec![("Sonnet".to_string(), 12.5), ("Fable".to_string(), 3.0)]);
    }

    #[test]
    fn keeps_the_percentage_when_a_line_has_no_reset_time() {
        let clock = &FixedOffsetClock::UTC;
        assert_eq!(
            parse_usage_output("Current session: 0% used", 0, clock),
            Some(ProbeResult { five_hour: Some(UsageWindow { used_pct: 0.0, resets_at: None }), seven_day: None, models: vec![] })
        );
    }

    #[test]
    fn returns_none_for_output_without_usage_lines() {
        let clock = &FixedOffsetClock::UTC;
        assert_eq!(parse_usage_output("Not logged in · Please run /login", 0, clock), None);
        assert_eq!(parse_usage_output("", 0, clock), None);
    }

    #[test]
    fn clamps_and_reads_odd_scopes() {
        let clock = &FixedOffsetClock::UTC;
        let out = "current SESSION: 140% used\nCurrent week ( ): 5% used\nCurrent week (ALL MODELS): 7% used";
        let result = parse_usage_output(out, 0, clock).unwrap();
        // an empty scope reads as the session, as `!scope` did
        assert_eq!(result.five_hour.map(|w| w.used_pct), Some(5.0));
        assert_eq!(result.seven_day.map(|w| w.used_pct), Some(7.0));
    }

    #[test]
    fn understands_midnight_and_noon() {
        for clock in &ZONES {
            let now = local(clock, 2026, 10, 7, 15, 20);
            assert_eq!(parse_reset_time("Oct 11, 12am (Europe/Istanbul)", now, clock), Some(local(clock, 2026, 10, 11, 0, 0)));
            assert_eq!(parse_reset_time("Oct 11, 12pm (Europe/Istanbul)", now, clock), Some(local(clock, 2026, 10, 11, 12, 0)));
            assert_eq!(parse_reset_time("Oct 11, 12:30am", now, clock), Some(local(clock, 2026, 10, 11, 0, 30)));
        }
    }

    #[test]
    fn takes_a_bare_time_as_today_or_tomorrow_once_it_has_passed() {
        for clock in &ZONES {
            let now = local(clock, 2026, 10, 7, 15, 20);
            assert_eq!(parse_reset_time("7:10pm (Europe/Istanbul)", now, clock), Some(local(clock, 2026, 10, 7, 19, 10)));
            assert_eq!(parse_reset_time("9am (Europe/Istanbul)", now, clock), Some(local(clock, 2026, 10, 8, 9, 0)));
            // within a minute it is still today
            assert_eq!(parse_reset_time("3:20pm", now + 59_000, clock), Some(now));
            // the last day of the year rolls into the next
            assert_eq!(parse_reset_time("1am", local(clock, 2026, 12, 31, 23, 0), clock), Some(local(clock, 2027, 1, 1, 1, 0)));
        }
    }

    #[test]
    fn picks_the_year_that_puts_the_reset_closest_to_now() {
        for clock in &ZONES {
            let now = local(clock, 2026, 10, 7, 15, 20);
            assert_eq!(parse_reset_time("Jan 2, 1pm", local(clock, 2026, 12, 30, 10, 0), clock), Some(local(clock, 2027, 1, 2, 13, 0)));
            assert_eq!(parse_reset_time("Dec 31, 11pm", local(clock, 2027, 1, 1, 0, 30), clock), Some(local(clock, 2026, 12, 31, 23, 0)));
            assert_eq!(parse_reset_time("Jan 2, 2027, 1pm", now, clock), Some(local(clock, 2027, 1, 2, 13, 0)));
        }
    }

    #[test]
    fn reads_long_month_names_and_upper_case() {
        let clock = &FixedOffsetClock::UTC;
        let now = local(clock, 2026, 10, 7, 15, 20);
        assert_eq!(parse_reset_time("October 11, 7:05PM", now, clock), Some(local(clock, 2026, 10, 11, 19, 5)));
        assert_eq!(parse_reset_time("Sept. 30 1am", now, clock), Some(local(clock, 2026, 9, 30, 1, 0)));
    }

    #[test]
    fn returns_none_for_text_it_does_not_understand() {
        let clock = &FixedOffsetClock::UTC;
        assert_eq!(parse_reset_time("soon", 0, clock), None);
        assert_eq!(parse_reset_time("Foo 3, 1pm", 0, clock), None);
        assert_eq!(parse_reset_time("7:10pmx", 0, clock), None);
    }
}
