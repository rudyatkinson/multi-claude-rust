//! Usage numbers of the accounts (`src/main/usage/*.ts` and `src/shared/usage.ts`):
//! - [`probe`]: what Claude Code's own `/usage` prints;
//! - [`normalize`]: what its status line sends, and how samples from several sources combine;
//! - [`view`]: how a limit is shown, which one is used up, which account to continue on;
//! - [`alerts`]: the few moments worth telling the user about;
//! - [`tap`]: the status line command that collects the samples;
//! - [`clock`]: local wall-clock time, which the reset times of `/usage` are written in.

pub mod alerts;
pub mod clock;
pub mod normalize;
pub mod probe;
pub mod tap;
pub mod view;

pub use alerts::{alert_message, AlertEvent, AlertMessage, LimitAlerts, TickResult, WindowKey, ALERT_THRESHOLDS};
pub use clock::{CivilDateTime, FixedOffsetClock, LocalClock};
pub use normalize::{
    apply_probe, merge_usage, parse_iso_ms, parse_session_meta, parse_status, parse_status_payload, SessionMeta, StatusSample,
    UsageSample, SAME_WINDOW_MS,
};
pub use probe::{parse_reset_time, parse_usage_output, ProbeResult, PROBE_ARGS};
pub use view::{account_initials, suggest_account, usage_view, used_up, AccountRef};

/// White space as JavaScript's `trim` knows it: WhiteSpace and LineTerminator.
pub(crate) fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0B}'
            | '\u{0C}'
            | '\r'
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

/// `s.trim()` in JavaScript.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `Math.round`: halves round up, also below zero.
pub(crate) fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_as_javascript() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(59.4), 59.0);
        assert_eq!(js_round(84.5), 85.0);
    }

    #[test]
    fn trims_as_javascript() {
        assert_eq!(js_trim("\u{FEFF} a b\u{3000}\n"), "a b");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
    }
}
