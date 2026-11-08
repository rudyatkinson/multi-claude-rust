//! The decision to move a limit-cut conversation (`decideSwitch` of `src/shared/autoSwitch.ts`).
//! The types and [`SWITCH_RULES`] are in [`crate::types`]; the timers and probes that feed this
//! decision belong to the backend.

use std::collections::HashSet;

use crate::types::{AuthKind, StayWhy, SwitchDecision, SwitchInput, SwitchMove, SwitchProfile, TurnKind, UsageState, UsageWindow, SWITCH_RULES};
use crate::usage::{suggest_account, usage_view, used_up};

fn none(why: &str) -> SwitchDecision {
    SwitchDecision::Nothing { why: why.to_string() }
}

/// Whether an account's numbers leave room for a conversation that arrives with all it holds:
/// nothing used up, the 5-hour limit at most 80%, the weekly one and that of the session's model
/// at most 94%.
fn has_room(state: Option<&UsageState>, now: i64, model: Option<&str>) -> bool {
    let Some(state) = state else {
        return false;
    };
    if used_up(Some(state), now, model).is_some() {
        return false;
    }
    let pct = |window: Option<&UsageWindow>| window.map_or(0, |w| usage_view(w, now).pct);
    let running = model.filter(|m| !m.is_empty()).map(str::to_lowercase);
    let own = running.and_then(|running| state.models.as_ref()?.iter().find(|m| running.contains(&m.name.to_lowercase())).map(|m| m.window()));
    pct(state.five_hour.as_ref()) <= SWITCH_RULES.five_hour_pct
        && pct(state.seven_day.as_ref()) <= SWITCH_RULES.week_pct
        && pct(own.as_ref()) <= SWITCH_RULES.week_pct
}

/// What to do about a turn that a usage limit seems to have cut off, when the app may move the
/// conversation by itself. Two things have to say that the limit is real: the session's hooks, and
/// numbers that were fetched after it failed. A rate limit error alone can be the server slowing
/// everyone down; 100% alone can be an account that goes on with extra usage.
///
/// The time is `input.now`.
pub fn decide_switch(input: &SwitchInput) -> SwitchDecision {
    let SwitchInput { session, now, usage, probed, .. } = input;
    let now = *now;
    if !input.enabled {
        return none("switched off");
    }
    let turn = match &session.turn {
        Some(turn) if matches!(turn.kind, TurnKind::Failed { limit: true, .. }) && turn.at == input.failed_at => turn,
        _ => return none("the turn went on"),
    };
    if !input.was_busy {
        return none("Claude was not at work");
    }
    if session.workflow {
        return none("a workflow step");
    }
    if session.moving == Some(true) {
        return none("already on its way");
    }
    if input.typed {
        return none("the user is at it");
    }
    let Some(conversation) = session.conversation_id.as_deref().filter(|c| !c.is_empty()) else {
        return none("no conversation");
    };
    if now - turn.at > SWITCH_RULES.fresh_ms {
        return none("too long ago");
    }

    let source = session.profile_id.as_str();
    let Some(&fresh) = probed.get(source) else {
        return SwitchDecision::Probe { profile_id: source.to_string() };
    };
    // a token account reports no usage, so nothing can confirm its limit
    if !fresh {
        return none("no fresh numbers");
    }
    let model = session.model.as_deref();
    let Some(limit) = used_up(usage.get(source), now, model) else {
        return none("the numbers show no full limit");
    };

    let mine: Vec<&SwitchMove> = input.moves.iter().filter(|m| m.conversation_id == conversation && now - m.at < SWITCH_RULES.memory_ms).collect();
    if mine.len() >= SWITCH_RULES.max_moves || mine.iter().any(|m| now - m.at < SWITCH_RULES.gap_ms) {
        return SwitchDecision::Stay { why: StayWhy::Moved, limit: limit.label };
    }

    // not back to where it came from, and not onto an account that is still taking another one in
    let left: HashSet<&str> = mine.iter().map(|m| m.from.as_str()).collect();
    let taking: HashSet<&str> = input.moves.iter().filter(|m| now - m.at < SWITCH_RULES.arrival_gap_ms).map(|m| m.to.as_str()).collect();
    let candidates: Vec<&SwitchProfile> = input
        .profiles
        .iter()
        .filter(|p| {
            let id = p.id.as_str();
            if id == source || !p.account.is_some_and(|a| a.logged_in) || p.auth == Some(AuthKind::Token) || left.contains(id) || taking.contains(id) {
                return false;
            }
            // numbers from before are good enough to choose whom to ask; the answer decides
            let state = usage.get(id);
            match probed.get(id) {
                Some(&fresh) => fresh && has_room(state, now, model),
                None => state.is_none() || has_room(state, now, model),
            }
        })
        .collect();
    let Some(best) = suggest_account(&candidates, usage, now, source, model) else {
        return SwitchDecision::Stay { why: StayWhy::Full, limit: limit.label };
    };
    if !probed.contains_key(&best.profile_id) {
        return SwitchDecision::Probe { profile_id: best.profile_id };
    }
    SwitchDecision::Move { to: best.profile_id, limit: limit.label }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::types::{LoginState, ModelUsage, StopError, SwitchSession, Turn};

    // Date.UTC(2026, 9, 8, 12)
    const NOW: i64 = 1_791_460_800_000;
    const LATER: i64 = NOW + 3_600_000;
    const CUT_AT: i64 = NOW - 5000;

    fn failed(at: i64, error: StopError, limit: bool) -> Turn {
        Turn { at, kind: TurnKind::Failed { error, text: None, limit, wait: None } }
    }

    fn cut_turn() -> Turn {
        failed(CUT_AT, StopError::RateLimit, true)
    }

    fn window(used_pct: f64, resets_at: i64) -> Option<UsageWindow> {
        Some(UsageWindow { used_pct, resets_at: Some(resets_at) })
    }

    fn full() -> UsageState {
        UsageState { five_hour: window(100.0, LATER), seven_day: window(40.0, LATER), ..UsageState::default() }
    }

    fn used(five_hour: f64, week: f64) -> UsageState {
        UsageState { five_hour: window(five_hour, LATER), seven_day: window(week, LATER), ..UsageState::default() }
    }

    fn used5(five_hour: f64) -> UsageState {
        used(five_hour, 10.0)
    }

    fn account(id: &str) -> SwitchProfile {
        SwitchProfile { id: id.into(), auth: None, account: Some(LoginState { logged_in: true }) }
    }

    fn usage(entries: &[(&str, UsageState)]) -> BTreeMap<String, UsageState> {
        entries.iter().map(|(id, state)| (id.to_string(), state.clone())).collect()
    }

    fn probed(entries: &[(&str, bool)]) -> BTreeMap<String, bool> {
        entries.iter().map(|(id, fresh)| (id.to_string(), *fresh)).collect()
    }

    fn session() -> SwitchSession {
        SwitchSession { profile_id: "main".into(), model: None, conversation_id: Some("c1".into()), turn: Some(cut_turn()), moving: None, workflow: false }
    }

    /// A conversation on `main` that a limit just cut off, with every number asked for and in.
    fn input() -> SwitchInput {
        SwitchInput {
            enabled: true,
            now: NOW,
            session: session(),
            failed_at: CUT_AT,
            was_busy: true,
            typed: false,
            profiles: vec![account("main"), account("elif"), account("emir")],
            usage: usage(&[("main", full()), ("elif", used5(30.0)), ("emir", used5(60.0))]),
            probed: probed(&[("main", true), ("elif", true), ("emir", true)]),
            moves: vec![],
        }
    }

    fn with(patch: impl FnOnce(&mut SwitchInput)) -> SwitchInput {
        let mut input = input();
        patch(&mut input);
        input
    }

    fn mv(conversation_id: &str, from: &str, to: &str, at: i64) -> SwitchMove {
        SwitchMove { conversation_id: conversation_id.into(), from: from.into(), to: to.into(), at }
    }

    /// `move()` of the TypeScript test: c1 came from emir an hour ago.
    fn came_back() -> SwitchMove {
        mv("c1", "emir", "main", NOW - 3_600_000)
    }

    fn moves_to(to: &str, limit: &str) -> SwitchDecision {
        SwitchDecision::Move { to: to.into(), limit: limit.into() }
    }

    fn probe(id: &str) -> SwitchDecision {
        SwitchDecision::Probe { profile_id: id.into() }
    }

    fn stays(why: StayWhy) -> SwitchDecision {
        SwitchDecision::Stay { why, limit: "5-hour limit".into() }
    }

    fn moved_to(decision: SwitchDecision) -> Option<String> {
        match decision {
            SwitchDecision::Move { to, .. } => Some(to),
            _ => None,
        }
    }

    // ---------- decideSwitch (test/autoSwitch.test.ts) ----------

    #[test]
    fn moves_to_the_account_with_the_most_room_once_the_numbers_are_in() {
        assert_eq!(decide_switch(&input()), moves_to("elif", "5-hour limit"));
    }

    #[test]
    fn asks_for_the_numbers_of_the_account_first_then_for_those_of_the_one_it_would_move_to() {
        assert_eq!(decide_switch(&with(|i| i.probed = probed(&[]))), probe("main"));
        assert_eq!(decide_switch(&with(|i| i.probed = probed(&[("main", true)]))), probe("elif"));
        assert_eq!(decide_switch(&with(|i| i.probed = probed(&[("main", true), ("elif", true)]))), moves_to("elif", "5-hour limit"));
    }

    #[test]
    fn does_nothing_unless_both_the_hooks_and_fresh_numbers_say_that_the_limit_is_reached() {
        let why = |patch: &dyn Fn(&mut SwitchInput)| match decide_switch(&with(patch)) {
            SwitchDecision::Nothing { why } => why,
            other => format!("{other:?}"),
        };
        // a rate limit error while the numbers show room: the server slowing everyone down
        assert_eq!(why(&|i| i.usage = usage(&[("main", used5(70.0)), ("elif", used5(30.0))])), "the numbers show no full limit");
        // no answer to the question, as for every token account
        assert_eq!(why(&|i| i.probed = probed(&[("main", false)])), "no fresh numbers");
        assert_eq!(why(&|i| i.session.turn = Some(failed(CUT_AT, StopError::Overloaded, false))), "the turn went on");
        assert_eq!(why(&|i| i.session.turn = Some(Turn { at: CUT_AT, kind: TurnKind::Done { background: false } })), "the turn went on");
        assert_eq!(why(&|i| i.session.turn = None), "the turn went on");
    }

    #[test]
    fn leaves_alone_what_is_not_its_business() {
        let why = |patch: &dyn Fn(&mut SwitchInput)| decide_switch(&with(patch));
        assert_eq!(why(&|i| i.enabled = false), none("switched off"));
        assert_eq!(why(&|i| i.was_busy = false), none("Claude was not at work"));
        assert_eq!(why(&|i| i.session.workflow = true), none("a workflow step"));
        assert_eq!(why(&|i| i.session.moving = Some(true)), none("already on its way"));
        assert_eq!(why(&|i| i.typed = true), none("the user is at it"));
        assert_eq!(why(&|i| i.session.conversation_id = None), none("no conversation"));
        // the same tab has failed again since, or gone on
        assert_eq!(why(&|i| i.failed_at = CUT_AT - 1), none("the turn went on"));
        assert_eq!(why(&|i| i.now = CUT_AT + SWITCH_RULES.fresh_ms + 1), none("too long ago"));
    }

    #[test]
    fn counts_the_weekly_limit_of_the_model_the_session_runs_on() {
        let fable = UsageState { models: Some(vec![ModelUsage { name: "Fable".into(), used_pct: 100.0, resets_at: Some(LATER) }]), ..used5(10.0) };
        let on_fable = with(|i| {
            i.session.model = Some("Fable 5 (1M context)".into());
            i.usage = usage(&[("main", fable.clone()), ("elif", fable.clone()), ("emir", used5(60.0))]);
        });
        // elif has the most room in its 5 hours, but none for this model
        assert_eq!(decide_switch(&on_fable), moves_to("emir", "Fable weekly limit"));
        let mut on_opus = on_fable.clone();
        on_opus.session.model = Some("Opus 5.5".into());
        assert!(matches!(decide_switch(&on_opus), SwitchDecision::Nothing { .. }));
    }

    #[test]
    fn wants_real_room_on_the_account_it_moves_to() {
        let room = |elif: UsageState, emir: UsageState| decide_switch(&with(|i| i.usage = usage(&[("main", full()), ("elif", elif), ("emir", emir)])));
        assert_eq!(room(used5(81.0), used5(100.0)), stays(StayWhy::Full));
        assert_eq!(moved_to(room(used5(80.0), used5(100.0))).as_deref(), Some("elif"));
        assert_eq!(moved_to(room(used(10.0, 95.0), used(20.0, 94.0))).as_deref(), Some("emir"));
        // a limit whose reset time has passed is empty again, whatever was last reported
        let reset = UsageState { five_hour: window(100.0, NOW - 1000), seven_day: window(10.0, LATER), ..UsageState::default() };
        assert_eq!(moved_to(room(reset, used5(100.0))).as_deref(), Some("elif"));
    }

    #[test]
    fn never_moves_to_an_account_that_is_logged_out_runs_on_a_token_or_gave_no_numbers() {
        let profiles = vec![
            account("main"),
            SwitchProfile { account: Some(LoginState { logged_in: false }), ..account("elif") },
            SwitchProfile { auth: Some(AuthKind::Token), ..account("emir") },
            account("nur"),
        ];
        let numbers = usage(&[("main", full()), ("elif", used5(1.0)), ("emir", used5(1.0)), ("nur", used5(50.0))]);
        let decide = |asked: &[(&str, bool)]| {
            decide_switch(&with(|i| {
                i.profiles = profiles.clone();
                i.usage = numbers.clone();
                i.probed = probed(asked);
            }))
        };
        assert_eq!(decide(&[("main", true)]), probe("nur"));
        assert_eq!(decide(&[("main", true), ("nur", false)]), stays(StayWhy::Full));
    }

    #[test]
    fn asks_an_account_it_knows_nothing_about_and_passes_it_by_when_no_numbers_come() {
        assert_eq!(
            decide_switch(&with(|i| {
                i.usage = usage(&[("main", full()), ("emir", used5(60.0))]);
                i.probed = probed(&[("main", true)]);
            })),
            probe("emir")
        );
        let two = |asked: &[(&str, bool)]| {
            decide_switch(&with(|i| {
                i.usage = usage(&[("main", full())]);
                i.profiles = vec![account("main"), account("elif")];
                i.probed = probed(asked);
            }))
        };
        assert_eq!(two(&[("main", true)]), probe("elif"));
        assert!(matches!(two(&[("main", true), ("elif", true)]), SwitchDecision::Stay { .. }));
    }

    #[test]
    fn takes_the_next_account_when_the_fresh_numbers_of_the_first_leave_no_room() {
        // elif looked best before it was asked
        let input = with(|i| {
            i.usage = usage(&[("main", full()), ("elif", used5(95.0)), ("emir", used5(60.0))]);
            i.probed = probed(&[("main", true), ("elif", true)]);
        });
        assert_eq!(decide_switch(&input), probe("emir"));
    }

    #[test]
    fn does_not_send_a_conversation_back_and_forth() {
        let roomy = || usage(&[("main", full()), ("elif", used5(70.0)), ("emir", used5(5.0))]);
        // it came from emir an hour ago: not back there
        let back = with(|i| {
            i.moves = vec![came_back()];
            i.usage = roomy();
        });
        assert_eq!(moved_to(decide_switch(&back)).as_deref(), Some("elif"));
        // moved a moment ago: it stays
        let just_now = with(|i| i.moves = vec![SwitchMove { at: NOW - SWITCH_RULES.gap_ms + 1000, ..came_back() }]);
        assert_eq!(decide_switch(&just_now), stays(StayWhy::Moved));
        // twice within the life of a 5-hour limit is enough
        let twice = with(|i| i.moves = vec![mv("c1", "elif", "emir", NOW - 3 * 3_600_000), came_back()]);
        assert_eq!(decide_switch(&twice), stays(StayWhy::Moved));
        // what is older than that no longer counts, and other conversations never did
        let old = with(|i| {
            i.moves = vec![SwitchMove { at: NOW - SWITCH_RULES.memory_ms - 1, ..came_back() }, mv("other", "nur", "x", NOW - 3_600_000)];
            i.usage = roomy();
        });
        assert_eq!(moved_to(decide_switch(&old)).as_deref(), Some("emir"));
    }

    #[test]
    fn sends_only_one_conversation_at_a_time_to_an_account() {
        let just_took = vec![mv("other", "main", "elif", NOW - 30_000)];
        assert_eq!(moved_to(decide_switch(&with(|i| i.moves = just_took.clone()))).as_deref(), Some("emir"));
        let alone = with(|i| {
            i.moves = just_took.clone();
            i.profiles = vec![account("main"), account("elif")];
        });
        assert_eq!(decide_switch(&alone), stays(StayWhy::Full));
        let awhile_ago = vec![mv("other", "main", "elif", NOW - SWITCH_RULES.arrival_gap_ms - 1)];
        assert_eq!(moved_to(decide_switch(&with(|i| i.moves = awhile_ago))).as_deref(), Some("elif"));
    }
}
