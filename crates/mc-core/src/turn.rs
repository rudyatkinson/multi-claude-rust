//! The turn state machine, `limit_cut`, `tab_mark` (`src/shared/turn.ts`).
//!
//! What a session's turn has come to, as its hooks report it, and what its tab shows for it. The
//! types are in [`crate::types::turn`]; this module holds the logic.

use std::sync::LazyLock;

use regex::Regex;

use crate::jsstr::cut16;
use crate::types::{AskWhat, HookSignal, LimitWaitStage, StopError, TabMark, Turn, TurnEvent, TurnKind, TurnWait};

/// The characters of JavaScript's `\s` and `String.prototype.trim`: Unicode White_Space without
/// U+0085, plus U+FEFF.
pub(crate) const JS_SPACE_CLASS: &str =
    r"[\t\n\x0B\x0C\r \u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}]";

/// Whether `c` is white space to JavaScript (`\s`, `trim`).
pub(crate) fn is_js_space(c: char) -> bool {
    c == '\u{FEFF}' || (c.is_whitespace() && c != '\u{0085}')
}

/// `s.trim()` in JavaScript.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

// a server that throttles everyone says so in these words; it is a rate limit, but not the account's
static NOT_THE_LIMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)not your usage limit").unwrap());

// "You've hit your session limit · resets 3:45pm", also for the week, a model and a spend limit.
// JavaScript's `\b` is an ASCII word boundary and its `.` stops at line terminators.
static LIMIT_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    let pattern = format!(
        r"(?i)^(?:api error:{JS_SPACE_CLASS}*)?(?:you['’]ve hit your(?-u:\b)[^\n\r\u{{2028}}\u{{2029}}]{{0,60}}(?-u:\b)limit(?-u:\b)|usage limit reached)"
    );
    Regex::new(&pattern).unwrap()
});

/// Whether a turn was cut off by a usage limit of the account, going by the error's name and by
/// the text Claude Code showed for it. A hint for what to offer; before anything is done by itself
/// the account's usage numbers have to say the same.
pub fn limit_cut(error: StopError, text: &str) -> bool {
    if NOT_THE_LIMIT.is_match(text) {
        return false;
    }
    error == StopError::RateLimit || LIMIT_TEXT.is_match(js_trim(text))
}

fn sentence(error: StopError) -> Option<&'static str> {
    Some(match error {
        StopError::RateLimit => "The account reached a usage limit.",
        StopError::Overloaded => "The Claude servers are overloaded.",
        StopError::AuthenticationFailed => "The account is no longer signed in.",
        StopError::OauthOrgNotAllowed => "The organisation of the account is not allowed here.",
        StopError::AccountOnHold => "The account is on hold.",
        StopError::BillingError => "The account has a billing problem.",
        StopError::InvalidRequest => "The request was refused as invalid.",
        StopError::ModelNotFound => "The model is not available to the account.",
        StopError::ServerError => "The Claude servers reported an error.",
        StopError::MaxOutputTokens => "The answer was cut off at the output limit.",
        StopError::CloudCredentialError => "The cloud credentials could not be loaded.",
        StopError::Unknown => return None,
    })
}

/// What to say about a turn that ended with an API error. `text` is what Claude Code showed for it.
pub fn stop_sentence(error: StopError, text: &str) -> String {
    if limit_cut(error, text) {
        return "The account reached a usage limit.".to_string();
    }
    if error == StopError::RateLimit {
        return "The Claude servers are limiting requests for the moment.".to_string();
    }
    if let Some(s) = sentence(error) {
        return s.to_string();
    }
    if text.is_empty() {
        "Claude stopped with an error.".to_string()
    } else {
        format!("Claude stopped with an error: {}", cut16(text, 200))
    }
}

/// Claude waits for an answer in the tab: a permission, a question or a plan.
fn asks(turn: Option<&Turn>) -> bool {
    matches!(turn.map(|t| &t.kind), Some(TurnKind::Permission | TurnKind::Question | TurnKind::Plan))
}

fn turn(at: i64, kind: TurnKind) -> Turn {
    Turn { at, kind }
}

/// The turn after an event. `None` when the event changes nothing (the TypeScript gives back the
/// very same object then); a session's turn never goes back to having none.
pub fn next_turn(prev: Option<&Turn>, event: &TurnEvent, now: i64) -> Option<Turn> {
    let prev_kind = prev.map(|t| &t.kind);
    match event {
        TurnEvent::Prompt => match prev_kind {
            Some(TurnKind::Working) => None,
            _ => Some(turn(now, TurnKind::Working)),
        },
        TurnEvent::Stop { background } => Some(turn(now, TurnKind::Done { background: *background })),
        // a plan is reported twice, before the tool runs and when it asks for approval
        TurnEvent::Plan => match prev_kind {
            Some(TurnKind::Plan) => None,
            _ => Some(turn(now, TurnKind::Plan)),
        },
        // a prompt that goes unanswered is reported again a few seconds later, less exactly
        TurnEvent::NeedsYou { what } => {
            if asks(prev) {
                return None;
            }
            let kind = match what {
                AskWhat::Permission => TurnKind::Permission,
                AskWhat::Question => TurnKind::Question,
            };
            Some(turn(now, kind))
        }
        TurnEvent::Failure { error, text } => Some(turn(
            now,
            TurnKind::Failed {
                error: *error,
                text: (!text.is_empty()).then(|| text.clone()),
                limit: limit_cut(*error, text),
                wait: None,
            },
        )),
        TurnEvent::LimitWait { stage } => {
            let wait = match stage {
                LimitWaitStage::Resumed => return Some(turn(now, TurnKind::Working)),
                LimitWaitStage::NeedsEnter => TurnWait::NeedsEnter,
                LimitWaitStage::Ended => TurnWait::Ended,
            };
            // only a usage limit is waited for, whatever the failure was called
            if let Some(Turn { at, kind: TurnKind::Failed { error, text, limit, wait: was } }) = prev {
                if *limit && *was == Some(wait) {
                    return None;
                }
                return Some(turn(*at, TurnKind::Failed { error: *error, text: text.clone(), limit: true, wait: Some(wait) }));
            }
            Some(turn(now, TurnKind::Failed { error: StopError::RateLimit, text: None, limit: true, wait: Some(wait) }))
        }
        // no hook says that a question was answered; what is typed there is taken for the answer
        TurnEvent::Typed => asks(prev).then(|| turn(now, TurnKind::Working)),
    }
}

/// The turn event a hook signal is. The TypeScript passed the signal itself (`noteTurn(id,
/// report.signal)`): the shapes overlap, and the texts were ignored.
pub fn turn_event_of(signal: &HookSignal) -> TurnEvent {
    match signal {
        HookSignal::Prompt => TurnEvent::Prompt,
        HookSignal::Stop { background, .. } => TurnEvent::Stop { background: *background },
        HookSignal::Plan { .. } => TurnEvent::Plan,
        HookSignal::Failure { error, text, .. } => TurnEvent::Failure { error: *error, text: text.clone() },
        HookSignal::NeedsYou { what } => TurnEvent::NeedsYou { what: *what },
        HookSignal::LimitWait { stage } => TurnEvent::LimitWait { stage: *stage },
    }
}

/// The name of a turn's kind, as it is on the wire.
pub fn kind_name(kind: &TurnKind) -> &'static str {
    match kind {
        TurnKind::Working => "working",
        TurnKind::Done { .. } => "done",
        TurnKind::Permission => "permission",
        TurnKind::Question => "question",
        TurnKind::Plan => "plan",
        TurnKind::Failed { .. } => "failed",
    }
}

/// Tells one state of a turn from another: what it is, when it got there, how a limit wait ended.
pub fn turn_key(turn: &Turn) -> String {
    let wait = match &turn.kind {
        TurnKind::Failed { wait: Some(TurnWait::NeedsEnter), .. } => "needsEnter",
        TurnKind::Failed { wait: Some(TurnWait::Ended), .. } => "ended",
        _ => "",
    };
    format!("{}:{}:{}", kind_name(&turn.kind), turn.at, wait)
}

/// Whether a turn has come to something the user should hear of however briefly Claude worked.
pub fn calls_for_you(turn: Option<&Turn>) -> bool {
    asks(turn) || matches!(turn.map(|t| &t.kind), Some(TurnKind::Failed { .. }))
}

/// The mark of a tab and what it means. The terminal title says whether Claude is working; the
/// turn says why it is not. Without a turn the title is all there is, as it was before tabs had
/// hooks.
pub fn tab_mark(exited: bool, turn: Option<&Turn>, busy: bool, busy_since: Option<i64>) -> (TabMark, String) {
    let mark = |state: TabMark, label: &str| (state, label.to_string());
    if exited {
        return mark(TabMark::Ended, "Ended");
    }
    let working = || mark(TabMark::Busy, "Working");
    let turn = match turn {
        None | Some(Turn { kind: TurnKind::Working | TurnKind::Done { .. }, .. }) => {
            if busy {
                return working();
            }
            let background = matches!(turn, Some(Turn { kind: TurnKind::Done { background: true }, .. }));
            let label = if background { "Waiting for work it started in the background" } else { "Waiting for input" };
            return mark(TabMark::Idle, label);
        }
        Some(turn) => turn,
    };
    // the title shows work that began after the turn got here, and no hook has told of it yet
    if busy && busy_since.is_some_and(|since| since > turn.at) {
        return working();
    }
    match &turn.kind {
        TurnKind::Permission => mark(TabMark::Ask, "Asks for a permission"),
        TurnKind::Question => mark(TabMark::Ask, "Asks a question"),
        TurnKind::Plan => mark(TabMark::Ask, "Presents a plan and waits for your approval"),
        TurnKind::Failed { error, text, limit, wait } => {
            if !*limit {
                return (TabMark::Failed, stop_sentence(*error, text.as_deref().unwrap_or("")));
            }
            match wait {
                Some(TurnWait::NeedsEnter) => mark(TabMark::Limit, "The usage limit has reset. Press Enter in the tab."),
                Some(TurnWait::Ended) => mark(TabMark::Limit, "Stopped at a usage limit. Claude Code no longer waits for it."),
                None => mark(TabMark::Limit, "Stopped at a usage limit"),
            }
        }
        // handled above
        TurnKind::Working | TurnKind::Done { .. } => working(),
    }
}

/// Whether Claude was at work in a turn, or about to be woken by work it had started.
pub fn in_progress(turn: Option<&Turn>) -> bool {
    matches!(turn.map(|t| &t.kind), Some(TurnKind::Working | TurnKind::Done { background: true })) || asks(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies events one after the other, each a second later than the one before.
    fn play(events: &[TurnEvent]) -> Option<Turn> {
        let mut current: Option<Turn> = None;
        for (i, event) in events.iter().enumerate() {
            if let Some(next) = next_turn(current.as_ref(), event, 1000 * (i as i64 + 1)) {
                current = Some(next);
            }
        }
        current
    }

    fn t(at: i64, kind: TurnKind) -> Turn {
        Turn { at, kind }
    }

    const PROMPT: TurnEvent = TurnEvent::Prompt;
    const STOP: TurnEvent = TurnEvent::Stop { background: false };
    const PERMISSION: TurnEvent = TurnEvent::NeedsYou { what: AskWhat::Permission };
    const QUESTION: TurnEvent = TurnEvent::NeedsYou { what: AskWhat::Question };
    const TYPED: TurnEvent = TurnEvent::Typed;
    const LIMIT_TEXT_SAMPLE: &str = "You've hit your session limit · resets 3:45pm";

    fn limit() -> TurnEvent {
        TurnEvent::Failure { error: StopError::RateLimit, text: LIMIT_TEXT_SAMPLE.into() }
    }
    fn wait(stage: LimitWaitStage) -> TurnEvent {
        TurnEvent::LimitWait { stage }
    }
    fn failed(at: i64, error: StopError, text: Option<&str>, limit: bool, wait: Option<TurnWait>) -> Turn {
        t(at, TurnKind::Failed { error, text: text.map(str::to_string), limit, wait })
    }

    // ---------- nextTurn ----------

    #[test]
    fn follows_a_turn_from_its_message_to_its_answer() {
        assert_eq!(play(&[PROMPT]), Some(t(1000, TurnKind::Working)));
        assert_eq!(play(&[PROMPT, STOP]), Some(t(2000, TurnKind::Done { background: false })));
        assert_eq!(play(&[PROMPT, TurnEvent::Stop { background: true }]), Some(t(2000, TurnKind::Done { background: true })));
    }

    #[test]
    fn keeps_the_moment_a_turn_began_when_more_messages_are_sent_into_it() {
        let working = play(&[PROMPT]).unwrap();
        assert_eq!(next_turn(Some(&working), &PROMPT, 5000), None);
    }

    #[test]
    fn says_what_claude_waits_for_and_goes_back_to_work_once_something_is_typed() {
        assert_eq!(play(&[PROMPT, PERMISSION]), Some(t(2000, TurnKind::Permission)));
        assert_eq!(play(&[PROMPT, QUESTION]), Some(t(2000, TurnKind::Question)));
        assert_eq!(play(&[PROMPT, TurnEvent::Plan]), Some(t(2000, TurnKind::Plan)));
        assert_eq!(play(&[PROMPT, PERMISSION, TYPED]), Some(t(3000, TurnKind::Working)));
        assert_eq!(play(&[PROMPT, TurnEvent::Plan, TYPED]), Some(t(3000, TurnKind::Working)));
    }

    #[test]
    fn takes_a_repeated_or_vaguer_report_of_the_same_wait_for_the_first_one() {
        let asked = play(&[PROMPT, QUESTION]).unwrap();
        // the notification that follows an unanswered question calls it a permission
        assert_eq!(next_turn(Some(&asked), &PERMISSION, 9000), None);
        let planned = play(&[PROMPT, TurnEvent::Plan]).unwrap();
        assert_eq!(next_turn(Some(&planned), &TurnEvent::Plan, 9000), None);
    }

    #[test]
    fn leaves_everything_else_alone_when_something_is_typed() {
        for turn in [None, play(&[PROMPT]), play(&[PROMPT, STOP]), play(&[PROMPT, limit()])] {
            assert_eq!(next_turn(turn.as_ref(), &TYPED, 9000), None);
        }
    }

    #[test]
    fn keeps_what_a_turn_failed_with_and_whether_a_usage_limit_cut_it_off() {
        assert_eq!(play(&[PROMPT, limit()]), Some(failed(2000, StopError::RateLimit, Some(LIMIT_TEXT_SAMPLE), true, None)));
        assert_eq!(
            play(&[PROMPT, TurnEvent::Failure { error: StopError::Overloaded, text: String::new() }]),
            Some(failed(2000, StopError::Overloaded, None, false, None))
        );
        let throttled = play(&[
            PROMPT,
            TurnEvent::Failure {
                error: StopError::RateLimit,
                text: "API Error: Server is temporarily limiting requests (not your usage limit)".into(),
            },
        ])
        .unwrap();
        assert!(matches!(throttled.kind, TurnKind::Failed { error: StopError::RateLimit, limit: false, .. }));
        let wire = serde_json::to_value(&throttled).unwrap();
        assert!(wire.get("limit").is_none());
    }

    #[test]
    fn follows_the_wait_of_claude_code_for_a_limit() {
        assert_eq!(play(&[PROMPT, limit(), wait(LimitWaitStage::Resumed)]), Some(t(3000, TurnKind::Working)));
        // the failure keeps its moment: it is still the same failure
        assert_eq!(
            play(&[PROMPT, limit(), wait(LimitWaitStage::NeedsEnter)]),
            Some(failed(2000, StopError::RateLimit, Some(LIMIT_TEXT_SAMPLE), true, Some(TurnWait::NeedsEnter)))
        );
        assert_eq!(
            play(&[PROMPT, limit(), wait(LimitWaitStage::Ended)]),
            Some(failed(2000, StopError::RateLimit, Some(LIMIT_TEXT_SAMPLE), true, Some(TurnWait::Ended)))
        );
        let waiting = play(&[PROMPT, limit(), wait(LimitWaitStage::Ended)]).unwrap();
        assert_eq!(next_turn(Some(&waiting), &wait(LimitWaitStage::Ended), 9000), None);
        // a different end of the wait is news
        assert_eq!(
            next_turn(Some(&waiting), &wait(LimitWaitStage::NeedsEnter), 9000),
            Some(failed(2000, StopError::RateLimit, Some(LIMIT_TEXT_SAMPLE), true, Some(TurnWait::NeedsEnter)))
        );
    }

    #[test]
    fn learns_of_a_limit_from_the_end_of_the_wait_when_the_failure_was_not_reported_as_one() {
        assert_eq!(
            play(&[PROMPT, STOP, wait(LimitWaitStage::NeedsEnter)]),
            Some(failed(3000, StopError::RateLimit, None, true, Some(TurnWait::NeedsEnter)))
        );
        assert_eq!(play(&[wait(LimitWaitStage::Ended)]), Some(failed(1000, StopError::RateLimit, None, true, Some(TurnWait::Ended))));
        let other = play(&[PROMPT, TurnEvent::Failure { error: StopError::Unknown, text: "odd".into() }, wait(LimitWaitStage::Ended)]);
        assert_eq!(other, Some(failed(2000, StopError::Unknown, Some("odd"), true, Some(TurnWait::Ended))));
    }

    #[test]
    fn starts_over_with_the_next_message() {
        assert_eq!(play(&[PROMPT, limit(), PROMPT]), Some(t(3000, TurnKind::Working)));
        assert_eq!(play(&[PROMPT, STOP, PROMPT]), Some(t(3000, TurnKind::Working)));
    }

    // ---------- limitCut ----------

    #[test]
    fn limit_cut_goes_by_the_name_of_the_error_and_by_the_words_claude_code_shows() {
        assert!(limit_cut(StopError::RateLimit, ""));
        assert!(limit_cut(StopError::RateLimit, "API Error: Rate limit reached"));
        assert!(limit_cut(StopError::Unknown, "You've hit your weekly limit · resets Mon 12:00am"));
        assert!(limit_cut(StopError::Unknown, "You’ve hit your Opus limit · resets 3:45pm"));
        assert!(limit_cut(StopError::BillingError, "You've hit your monthly spend limit · raise it at claude.ai/settings/usage"));
        assert!(limit_cut(StopError::Unknown, "Usage limit reached · limit resets 3:45pm"));
        assert!(limit_cut(StopError::Unknown, "  api error:  you've HIT YOUR limit"));
    }

    #[test]
    fn limit_cut_is_not_taken_in_by_other_errors() {
        assert!(!limit_cut(StopError::RateLimit, "API Error: Server is temporarily limiting requests (not your usage limit)"));
        assert!(!limit_cut(StopError::Overloaded, "API Error: Repeated 529 Overloaded errors."));
        assert!(!limit_cut(StopError::Unknown, ""));
        assert!(!limit_cut(StopError::Unknown, "The plan says: you have hit your limit of patience"));
        assert!(!limit_cut(StopError::ServerError, "API Error: Request rejected (429)"));
        // the words must be whole, and on one line
        assert!(!limit_cut(StopError::Unknown, "You've hit yourself against the limits"));
        assert!(!limit_cut(StopError::Unknown, "You've hit your\nlimit"));
    }

    // ---------- stopSentence, turnKey, callsForYou, inProgress ----------

    #[test]
    fn stop_sentences() {
        assert_eq!(stop_sentence(StopError::Unknown, ""), "Claude stopped with an error.");
        assert_eq!(stop_sentence(StopError::Unknown, "boom"), "Claude stopped with an error: boom");
        let long = "y".repeat(500);
        assert_eq!(stop_sentence(StopError::Unknown, &long), format!("Claude stopped with an error: {}", "y".repeat(200)));
        assert_eq!(stop_sentence(StopError::RateLimit, "(not your usage limit)"), "The Claude servers are limiting requests for the moment.");
        assert_eq!(stop_sentence(StopError::ModelNotFound, ""), "The model is not available to the account.");
    }

    #[test]
    fn turn_keys() {
        assert_eq!(turn_key(&t(5, TurnKind::Working)), "working:5:");
        assert_eq!(turn_key(&failed(7, StopError::RateLimit, None, true, None)), "failed:7:");
        assert_eq!(turn_key(&failed(7, StopError::RateLimit, None, true, Some(TurnWait::NeedsEnter))), "failed:7:needsEnter");
        assert_eq!(turn_key(&failed(7, StopError::RateLimit, None, true, Some(TurnWait::Ended))), "failed:7:ended");
    }

    #[test]
    fn calls_for_you_and_in_progress() {
        assert!(!calls_for_you(None));
        assert!(!calls_for_you(Some(&t(1, TurnKind::Working))));
        assert!(!calls_for_you(Some(&t(1, TurnKind::Done { background: false }))));
        assert!(calls_for_you(Some(&t(1, TurnKind::Plan))));
        assert!(calls_for_you(Some(&failed(1, StopError::Overloaded, None, false, None))));

        assert!(!in_progress(None));
        assert!(in_progress(Some(&t(1, TurnKind::Working))));
        assert!(in_progress(Some(&t(1, TurnKind::Question))));
        assert!(in_progress(Some(&t(1, TurnKind::Done { background: true }))));
        assert!(!in_progress(Some(&t(1, TurnKind::Done { background: false }))));
        assert!(!in_progress(Some(&failed(1, StopError::RateLimit, None, true, None))));
    }

    // ---------- tabMark (test/sessionAlerts.test.ts) ----------

    fn limit_turn() -> Turn {
        failed(5, StopError::RateLimit, None, true, None)
    }
    fn mark(turn: Option<&Turn>, busy: bool, busy_since: Option<i64>) -> TabMark {
        tab_mark(false, turn, busy, busy_since).0
    }
    fn labelled(state: TabMark, label: &str) -> (TabMark, String) {
        (state, label.to_string())
    }

    #[test]
    fn tab_mark_goes_by_the_terminal_title_alone_for_a_session_without_hooks() {
        assert_eq!(tab_mark(false, None, true, None), labelled(TabMark::Busy, "Working"));
        assert_eq!(tab_mark(false, None, false, None), labelled(TabMark::Idle, "Waiting for input"));
        assert_eq!(tab_mark(true, Some(&limit_turn()), true, None), labelled(TabMark::Ended, "Ended"));
    }

    #[test]
    fn tab_mark_leaves_working_and_finished_turns_to_the_title_as_well() {
        assert_eq!(mark(Some(&t(5, TurnKind::Working)), true, None), TabMark::Busy);
        assert_eq!(mark(Some(&t(5, TurnKind::Working)), false, None), TabMark::Idle);
        assert_eq!(mark(Some(&t(5, TurnKind::Done { background: false })), true, Some(1)), TabMark::Busy);
        assert_eq!(mark(Some(&t(5, TurnKind::Done { background: false })), false, None), TabMark::Idle);
        assert_eq!(
            tab_mark(false, Some(&t(5, TurnKind::Done { background: true })), false, None).1,
            "Waiting for work it started in the background"
        );
    }

    #[test]
    fn tab_mark_says_what_claude_waits_for_and_what_a_turn_failed_with() {
        assert_eq!(tab_mark(false, Some(&t(5, TurnKind::Permission)), false, None), labelled(TabMark::Ask, "Asks for a permission"));
        assert_eq!(tab_mark(false, Some(&t(5, TurnKind::Question)), false, None), labelled(TabMark::Ask, "Asks a question"));
        assert_eq!(
            tab_mark(false, Some(&t(5, TurnKind::Plan)), false, None),
            labelled(TabMark::Ask, "Presents a plan and waits for your approval")
        );
        assert_eq!(
            tab_mark(false, Some(&failed(5, StopError::Overloaded, None, false, None)), false, None),
            labelled(TabMark::Failed, "The Claude servers are overloaded.")
        );
        assert_eq!(tab_mark(false, Some(&limit_turn()), false, None), labelled(TabMark::Limit, "Stopped at a usage limit"));
        let needs_enter = failed(5, StopError::RateLimit, None, true, Some(TurnWait::NeedsEnter));
        assert_eq!(tab_mark(false, Some(&needs_enter), false, None).1, "The usage limit has reset. Press Enter in the tab.");
        let ended = failed(5, StopError::RateLimit, None, true, Some(TurnWait::Ended));
        assert!(tab_mark(false, Some(&ended), false, None).1.contains("no longer waits"));
    }

    #[test]
    fn tab_mark_believes_a_spinning_title_from_before_but_not_one_that_began_afterwards() {
        // Claude Code may keep its mark turning while it asks
        assert_eq!(mark(Some(&t(5, TurnKind::Permission)), true, Some(1)), TabMark::Ask);
        assert_eq!(mark(Some(&limit_turn()), true, Some(5)), TabMark::Limit);
        // answered with the mouse, or continued by Claude Code itself: no hook told of it
        assert_eq!(mark(Some(&t(5, TurnKind::Permission)), true, Some(6)), TabMark::Busy);
        assert_eq!(mark(Some(&limit_turn()), true, Some(6)), TabMark::Busy);
        assert_eq!(mark(Some(&limit_turn()), true, None), TabMark::Limit);
    }

    #[test]
    fn a_hook_signal_is_the_turn_event_of_the_same_shape() {
        assert_eq!(turn_event_of(&HookSignal::Prompt), TurnEvent::Prompt);
        assert_eq!(turn_event_of(&HookSignal::Stop { text: "x".into(), background: true }), TurnEvent::Stop { background: true });
        assert_eq!(turn_event_of(&HookSignal::Plan { text: "p".into() }), TurnEvent::Plan);
        assert_eq!(
            turn_event_of(&HookSignal::Failure { reason: "r".into(), error: StopError::Overloaded, text: "t".into() }),
            TurnEvent::Failure { error: StopError::Overloaded, text: "t".into() }
        );
        assert_eq!(turn_event_of(&HookSignal::NeedsYou { what: AskWhat::Question }), QUESTION);
        assert_eq!(
            turn_event_of(&HookSignal::LimitWait { stage: LimitWaitStage::Ended }),
            TurnEvent::LimitWait { stage: LimitWaitStage::Ended }
        );
    }

    #[test]
    fn js_trim_matches_javascript() {
        assert_eq!(js_trim("\u{FEFF} a \u{3000}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }
}
