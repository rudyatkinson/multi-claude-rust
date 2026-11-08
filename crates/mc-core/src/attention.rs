//! `should_alert` (`src/shared/attention.ts`).

/// Work shorter than this is not worth an alert: the user was probably still looking.
pub const MIN_BUSY_MS: i64 = 3000;

/// Whether a session that just stopped working should call for attention. It does when the user
/// is not looking at it: its tab is in the background, or the whole window is.
///
/// `busy_since` is when the session began to work (ms); without it the work counts as having
/// taken no time, as `now - (busySince ?? now)` did in the window.
pub fn should_alert(busy_since: Option<i64>, now: i64, tab_visible: bool, window_focused: bool) -> bool {
    let busy_ms = now - busy_since.unwrap_or(now);
    busy_ms >= MIN_BUSY_MS && !(tab_visible && window_focused)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(busy_ms: i64, tab_active: bool, window_focused: bool) -> bool {
        should_alert(Some(100_000 - busy_ms), 100_000, tab_active, window_focused)
    }

    #[test]
    fn alerts_for_real_work_that_finished_out_of_sight() {
        assert!(alert(20_000, false, true));
        assert!(alert(20_000, true, false));
        assert!(alert(3000, false, false));
    }

    #[test]
    fn stays_quiet_when_the_user_is_looking_at_the_tab_or_the_work_was_a_blip() {
        assert!(!alert(20_000, true, true));
        assert!(!alert(800, false, false));
        assert!(!alert(2999, false, false));
        assert!(!should_alert(None, 100_000, false, false));
    }
}
