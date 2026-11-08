//! When an idle Claude tab is put to sleep: its process ends, the tab keeps its place, its name,
//! its output and its conversation, and comes back with `claude --resume` when it is woken. The
//! backend asks [`should_sleep`] every 30 s for every session (`src-tauri/src/sleep.rs`).
//!
//! # The idle rule
//!
//! A session sleeps once it has been quiet for [`SleepInput::after_minutes`], where quiet is
//! measured from [`quiet_since`]:
//!
//! - **The turn is `done`** (Claude finished its answer, nothing runs in the background): from
//!   the later of the last input and the moment the turn ended. Output after that does not keep
//!   the tab awake: Claude Code redraws now and then (its status line, a resize) without anything
//!   happening. Typing does, even before it is sent: a draft in the prompt is someone at work.
//! - **No turn** (a session without hooks, or one that has not been sent a message yet): from the
//!   later of the last input and the last output. The output is all there is to go by.
//!
//! Every other turn keeps the tab awake whatever the clock says: `working` and `done` with work in
//! the background ([`in_progress`]), and a permission, a question, a plan or a failed turn
//! ([`calls_for_you`]), which wait for the user. So does being a workflow step, being on the way
//! to another account, the user's "Keep awake", and being the tab the user looks at (in front in
//! the area that has the keyboard while the window has the focus). Login tabs never sleep, and
//! `0` minutes switches the whole thing off.

use crate::turn::{calls_for_you, in_progress};
use crate::types::{SessionKind, SessionStatus, Turn, TurnKind};

/// How often the backend looks for idle tabs.
pub const SLEEP_CHECK_MS: u64 = 30_000;
/// How long a session that is put to sleep gets to end by itself before it is killed.
pub const SLEEP_END_MS: u64 = 3_000;

/// What [`should_sleep`] needs to know of one session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepInput<'a> {
    pub kind: SessionKind,
    pub status: SessionStatus,
    /// What the session's hooks say its turn has come to.
    pub turn: Option<&'a Turn>,
    /// When something was last typed into the tab (ms since the epoch); its start before that.
    pub last_input_at: i64,
    /// When the session last printed something; its start before that.
    pub last_output_at: i64,
    /// On its way to another account, restarting, or already being put to sleep.
    pub moving: bool,
    /// The user's "Keep awake".
    pub keep_awake: bool,
    /// A workflow run waits for this tab.
    pub workflow_step: bool,
    /// The user is looking at it: the tab in front in the area that has the keyboard, while the
    /// window has the focus.
    pub watched: bool,
    /// `sleepIdleMinutes` as it is used; 0 never.
    pub after_minutes: u32,
}

/// The moment from which the session counts as quiet (see the module's idle rule).
pub fn quiet_since(input: &SleepInput<'_>) -> i64 {
    match input.turn {
        Some(Turn { at, kind: TurnKind::Done { background: false } }) => input.last_input_at.max(*at),
        _ => input.last_input_at.max(input.last_output_at),
    }
}

/// Whether the session should be put to sleep now (see the module's idle rule).
pub fn should_sleep(input: SleepInput<'_>, now: i64) -> bool {
    if input.after_minutes == 0 || input.kind != SessionKind::Claude || input.status != SessionStatus::Running {
        return false;
    }
    if input.moving || input.keep_awake || input.workflow_step || input.watched {
        return false;
    }
    if in_progress(input.turn) || calls_for_you(input.turn) {
        return false;
    }
    now - quiet_since(&input) >= i64::from(input.after_minutes) * 60_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StopError, TurnWait};

    const MIN: i64 = 60_000;
    /// "now" in the tests: an hour after the session started at 0.
    const NOW: i64 = 60 * MIN;

    fn input(turn: Option<&Turn>) -> SleepInput<'_> {
        SleepInput {
            kind: SessionKind::Claude,
            status: SessionStatus::Running,
            turn,
            last_input_at: 0,
            last_output_at: 0,
            moving: false,
            keep_awake: false,
            workflow_step: false,
            watched: false,
            after_minutes: 15,
        }
    }

    fn turn(at: i64, kind: TurnKind) -> Turn {
        Turn { at, kind }
    }

    fn failed(limit: bool, wait: Option<TurnWait>) -> TurnKind {
        TurnKind::Failed { error: StopError::RateLimit, text: None, limit, wait }
    }

    #[test]
    fn a_tab_that_nobody_touched_sleeps_once_the_time_is_up() {
        assert!(should_sleep(input(None), NOW));
        // exactly at the limit, and a moment before it
        let fresh = SleepInput { last_input_at: NOW - 15 * MIN, last_output_at: NOW - 15 * MIN, ..input(None) };
        assert!(should_sleep(fresh, NOW));
        assert!(!should_sleep(fresh, NOW - 1));
    }

    #[test]
    fn every_exclusion_keeps_a_quiet_tab_awake() {
        let done = turn(0, TurnKind::Done { background: false });
        let quiet = input(Some(&done));
        assert!(should_sleep(quiet, NOW));
        let cases: [(&str, SleepInput<'_>); 9] = [
            ("switched off", SleepInput { after_minutes: 0, ..quiet }),
            ("a login tab", SleepInput { kind: SessionKind::Login, ..quiet }),
            ("ended", SleepInput { status: SessionStatus::Exited, ..quiet }),
            ("asleep already", SleepInput { status: SessionStatus::Sleeping, ..quiet }),
            ("moving", SleepInput { moving: true, ..quiet }),
            ("kept awake", SleepInput { keep_awake: true, ..quiet }),
            ("a workflow step", SleepInput { workflow_step: true, ..quiet }),
            ("being looked at", SleepInput { watched: true, ..quiet }),
            ("typed into a minute ago", SleepInput { last_input_at: NOW - MIN, ..quiet }),
        ];
        for (why, case) in cases {
            assert!(!should_sleep(case, NOW), "{why}");
        }
    }

    #[test]
    fn a_turn_under_way_or_one_that_waits_for_the_user_keeps_it_awake() {
        let kinds = [
            ("working", TurnKind::Working),
            ("background work", TurnKind::Done { background: true }),
            ("a permission", TurnKind::Permission),
            ("a question", TurnKind::Question),
            ("a plan", TurnKind::Plan),
            ("an API error", TurnKind::Failed { error: StopError::Overloaded, text: None, limit: false, wait: None }),
            ("a usage limit", failed(true, None)),
            ("a limit that has reset", failed(true, Some(TurnWait::NeedsEnter))),
            ("a limit wait that ended", failed(true, Some(TurnWait::Ended))),
        ];
        for (why, kind) in kinds {
            let t = turn(0, kind);
            assert!(!should_sleep(input(Some(&t)), NOW), "{why}");
        }
    }

    #[test]
    fn after_a_finished_turn_only_typing_and_the_turn_count_not_redraws() {
        let done = turn(NOW - 20 * MIN, TurnKind::Done { background: false });
        // Claude Code redrew a minute ago: the tab still sleeps
        let redrawn = SleepInput { last_output_at: NOW - MIN, ..input(Some(&done)) };
        assert!(should_sleep(redrawn, NOW));
        assert_eq!(quiet_since(&redrawn), NOW - 20 * MIN);
        // a draft typed five minutes ago keeps it awake
        let typed = SleepInput { last_input_at: NOW - 5 * MIN, ..redrawn };
        assert!(!should_sleep(typed, NOW));
        assert_eq!(quiet_since(&typed), NOW - 5 * MIN);
        // the turn itself ended only ten minutes ago
        let recent = turn(NOW - 10 * MIN, TurnKind::Done { background: false });
        assert!(!should_sleep(input(Some(&recent)), NOW));
    }

    #[test]
    fn without_a_turn_output_counts_as_activity() {
        let printing = SleepInput { last_output_at: NOW - MIN, ..input(None) };
        assert!(!should_sleep(printing, NOW));
        assert_eq!(quiet_since(&printing), NOW - MIN);
        let typed = SleepInput { last_input_at: NOW - 2 * MIN, ..input(None) };
        assert!(!should_sleep(typed, NOW));
        // the minutes are the setting's
        assert!(should_sleep(SleepInput { after_minutes: 1, ..printing }, NOW));
        assert!(!should_sleep(SleepInput { after_minutes: 1440, ..input(None) }, NOW));
    }
}
