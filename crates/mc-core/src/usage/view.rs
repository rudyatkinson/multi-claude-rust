//! How usage is shown and what it means for an account (`src/shared/usage.ts`, and
//! `accountInitials` of `src/renderer/src/format.ts`).

use std::collections::BTreeMap;

use super::{js_round, js_trim};
use crate::types::{Profile, ProfileView, Suggestion, SwitchProfile, UsageState, UsageTone, UsageView, UsageWindow, UsedUp};

/// How a usage window is shown at `now`: on the account card, in the collapsed sidebar and in the
/// tray menu alike.
pub fn usage_view(window: &UsageWindow, now: i64) -> UsageView {
    let reset = window.resets_at.is_some_and(|at| now >= at);
    let pct = if reset { 0 } else { js_round(window.used_pct) as i64 };
    let tone = if pct >= 85 {
        UsageTone::Red
    } else if pct >= 60 {
        UsageTone::Amber
    } else {
        UsageTone::Ok
    };
    UsageView { pct, tone, reset }
}

fn full(window: Option<&UsageWindow>, now: i64) -> bool {
    window.is_some_and(|w| usage_view(w, now).pct >= 100)
}

/// The limit of an account that is at 100% and has not reset, if any: the 5-hour one, the weekly
/// one, or the weekly limit of the model a session runs on (`model` is the session's model as
/// Claude Code names it, e.g. "Fable 5 (1M context)").
///
/// It is a strong hint, not proof: an account with extra usage enabled keeps working at 100%.
pub fn used_up(usage: Option<&UsageState>, now: i64, model: Option<&str>) -> Option<UsedUp> {
    let usage = usage?;
    if full(usage.five_hour.as_ref(), now) {
        return Some(UsedUp { label: "5-hour limit".into(), resets_at: usage.five_hour.as_ref().and_then(|w| w.resets_at) });
    }
    if full(usage.seven_day.as_ref(), now) {
        return Some(UsedUp { label: "weekly limit".into(), resets_at: usage.seven_day.as_ref().and_then(|w| w.resets_at) });
    }
    let running = model.filter(|m| !m.is_empty())?.to_lowercase();
    let own = usage.models.as_ref()?.iter().find(|m| running.contains(&m.name.to_lowercase()) && full(Some(&m.window()), now))?;
    Some(UsedUp { label: format!("{} weekly limit", own.name), resets_at: own.resets_at })
}

/// As much of an account as [`suggest_account`] needs: `{ id, account?: { loggedIn } }`.
pub trait AccountRef {
    fn account_id(&self) -> &str;
    fn logged_in(&self) -> bool;
}

impl AccountRef for Profile {
    fn account_id(&self) -> &str {
        &self.id
    }
    fn logged_in(&self) -> bool {
        self.account.as_ref().is_some_and(|a| a.logged_in)
    }
}

impl AccountRef for ProfileView {
    fn account_id(&self) -> &str {
        &self.id
    }
    fn logged_in(&self) -> bool {
        self.account.as_ref().is_some_and(|a| a.logged_in)
    }
}

impl AccountRef for SwitchProfile {
    fn account_id(&self) -> &str {
        &self.id
    }
    fn logged_in(&self) -> bool {
        self.account.is_some_and(|a| a.logged_in)
    }
}

impl<T: AccountRef + ?Sized> AccountRef for &T {
    fn account_id(&self) -> &str {
        (**self).account_id()
    }
    fn logged_in(&self) -> bool {
        (**self).logged_in()
    }
}

/// The account to continue on when `exclude_id` has run out: logged in, not used up itself, and
/// with the most room in its 5-hour limit. Accounts without usage numbers come last. `model` is
/// the model the session runs on: an account whose weekly limit for that model is full is no help.
pub fn suggest_account<P: AccountRef>(
    profiles: &[P],
    usage: &BTreeMap<String, UsageState>,
    now: i64,
    exclude_id: &str,
    model: Option<&str>,
) -> Option<Suggestion> {
    struct Candidate<'a> {
        profile_id: &'a str,
        five_hour_pct: Option<i64>,
        /// What the 5-hour number counts for in the order: a plan without a 5-hour limit (Ultra)
        /// reports weekly numbers only, and no limit counts as all the room.
        five_rank: Option<i64>,
        week_pct: Option<i64>,
    }
    let mut candidates: Vec<Candidate> = profiles
        .iter()
        .filter(|p| p.account_id() != exclude_id && p.logged_in() && used_up(usage.get(p.account_id()), now, model).is_none())
        .map(|p| {
            let state = usage.get(p.account_id());
            let five_hour_pct = state.and_then(|s| s.five_hour.as_ref()).map(|w| usage_view(w, now).pct);
            let has_weekly = state.is_some_and(|s| s.seven_day.is_some() || s.models.as_ref().is_some_and(|m| !m.is_empty()));
            Candidate {
                profile_id: p.account_id(),
                five_hour_pct,
                five_rank: if five_hour_pct.is_none() && has_weekly { Some(0) } else { five_hour_pct },
                week_pct: state.and_then(|s| s.seven_day.as_ref()).map(|w| usage_view(w, now).pct),
            }
        })
        .collect();
    // unknown counts as worse than any known number; the week breaks ties
    let rank = |pct: Option<i64>| pct.unwrap_or(101);
    candidates.sort_by(|a, b| rank(a.five_rank).cmp(&rank(b.five_rank)).then(rank(a.week_pct).cmp(&rank(b.week_pct))));
    candidates.first().map(|best| Suggestion { profile_id: best.profile_id.to_string(), five_hour_pct: best.five_hour_pct })
}

/// The letters that stand for an account in the collapsed sidebar: its initial, or its first two
/// letters when another account starts with the same one ("El" and "Em").
pub fn account_initials<S: AsRef<str>>(name: &str, all_names: &[S]) -> String {
    let mut letters = js_trim(name).chars();
    let first = letters.next().map_or_else(|| "?".to_string(), |c| c.to_uppercase().collect());
    let shared = all_names
        .iter()
        .filter(|other| js_trim(other.as_ref()).chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default() == first)
        .count()
        > 1;
    match letters.next() {
        Some(second) if shared => first + &second.to_lowercase().collect::<String>(),
        _ => first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LoginState, ModelUsage};

    // ---------- usageView (test/probe.test.ts) ----------

    const T: i64 = 1_000_000_000_000;

    fn w(used_pct: f64, resets_at: Option<i64>) -> UsageWindow {
        UsageWindow { used_pct, resets_at }
    }

    #[test]
    fn colours_the_bar_by_how_full_the_window_is() {
        assert_eq!(usage_view(&w(59.4, Some(T + 1)), T), UsageView { pct: 59, tone: UsageTone::Ok, reset: false });
        assert_eq!(usage_view(&w(60.0, Some(T + 1)), T), UsageView { pct: 60, tone: UsageTone::Amber, reset: false });
        assert_eq!(usage_view(&w(84.4, Some(T + 1)), T).tone, UsageTone::Amber);
        assert_eq!(usage_view(&w(85.0, Some(T + 1)), T).tone, UsageTone::Red);
    }

    #[test]
    fn shows_an_empty_window_once_the_reset_time_has_passed() {
        assert_eq!(usage_view(&w(92.0, Some(T - 1)), T), UsageView { pct: 0, tone: UsageTone::Ok, reset: true });
        assert!(usage_view(&w(92.0, Some(T)), T).reset);
    }

    #[test]
    fn keeps_the_reported_percentage_when_the_reset_time_is_unknown() {
        assert_eq!(usage_view(&w(92.0, None), T), UsageView { pct: 92, tone: UsageTone::Red, reset: false });
    }

    // ---------- accountInitials (test/probe.test.ts) ----------

    #[test]
    fn uses_one_letter_when_it_is_unique_and_two_when_accounts_share_it() {
        let names = ["Bilal", "Elif", "Emir"];
        assert_eq!(account_initials("Bilal", &names), "B");
        assert_eq!(account_initials("Elif", &names), "El");
        assert_eq!(account_initials("Emir", &names), "Em");
    }

    #[test]
    fn ignores_case_and_surrounding_spaces_and_copes_with_very_short_names() {
        assert_eq!(account_initials(" work ", &["work", "Web"]), "Wo");
        assert_eq!(account_initials("X", &["X", "x-ray"]), "X");
        assert_eq!(account_initials("", &[""]), "?");
    }

    // ---------- usedUp / suggestAccount (test/conversations.test.ts) ----------

    // Date.UTC(2026, 9, 7, 12)
    const NOW: i64 = 1_791_374_400_000;
    const LATER: i64 = NOW + 3_600_000;

    fn win(used_pct: f64) -> Option<UsageWindow> {
        Some(w(used_pct, Some(LATER)))
    }

    fn state(five_hour: Option<f64>, seven_day: Option<f64>) -> UsageState {
        UsageState { five_hour: five_hour.and_then(win), seven_day: seven_day.and_then(win), models: None, observed_at: None }
    }

    fn fable(used_pct: f64) -> Option<Vec<ModelUsage>> {
        Some(vec![ModelUsage { name: "Fable".into(), used_pct, resets_at: Some(LATER) }])
    }

    #[test]
    fn names_the_limit_that_is_at_100_percent_the_5_hour_one_first() {
        let up = |label: &str| Some(UsedUp { label: label.into(), resets_at: Some(LATER) });
        assert_eq!(used_up(Some(&state(Some(100.0), Some(100.0))), NOW, None), up("5-hour limit"));
        assert_eq!(used_up(Some(&state(Some(40.0), Some(99.6))), NOW, None), up("weekly limit"));
        assert_eq!(used_up(Some(&state(Some(99.4), None)), NOW, None), None);
        assert_eq!(used_up(None, NOW, None), None);
    }

    #[test]
    fn does_not_count_a_limit_whose_reset_time_has_passed() {
        let usage = UsageState { five_hour: Some(w(100.0, Some(NOW - 1000))), ..UsageState::default() };
        assert_eq!(used_up(Some(&usage), NOW, None), None);
    }

    #[test]
    fn counts_a_model_limit_only_for_a_session_that_runs_on_that_model() {
        let usage = UsageState { models: fable(100.0), ..state(Some(10.0), None) };
        assert_eq!(
            used_up(Some(&usage), NOW, Some("Fable 5 (1M context)")),
            Some(UsedUp { label: "Fable weekly limit".into(), resets_at: Some(LATER) })
        );
        assert_eq!(used_up(Some(&usage), NOW, Some("Opus 5.5")), None);
        assert_eq!(used_up(Some(&usage), NOW, None), None);
    }

    fn account(id: &str, logged_in: bool) -> SwitchProfile {
        SwitchProfile { id: id.into(), auth: None, account: Some(LoginState { logged_in }) }
    }

    #[test]
    fn suggests_the_logged_in_account_with_the_most_room() {
        let profiles =
            [account("main", true), account("elif", true), account("emir", true), account("token", true), account("out", false)];
        let mut usage = BTreeMap::from([
            ("main".to_string(), state(Some(100.0), None)),
            ("elif".to_string(), state(Some(55.0), Some(20.0))),
            ("emir".to_string(), state(Some(12.0), Some(80.0))),
            ("out".to_string(), state(Some(0.0), None)),
        ]);
        let suggestion = |id: &str, pct: Option<i64>| Some(Suggestion { profile_id: id.into(), five_hour_pct: pct });
        assert_eq!(suggest_account(&profiles, &usage, NOW, "main", None), suggestion("emir", Some(12)));
        // an account that is used up itself is no help
        usage.insert("emir".into(), state(Some(12.0), Some(100.0)));
        assert_eq!(suggest_account(&profiles, &usage, NOW, "main", None), suggestion("elif", Some(55)));
        usage.insert("emir".into(), state(Some(12.0), Some(80.0)));
        // only an account without numbers is left
        assert_eq!(suggest_account(&profiles[3..], &usage, NOW, "main", None), suggestion("token", None));
        assert_eq!(suggest_account(&profiles[..1], &usage, NOW, "main", None), None);
    }

    #[test]
    fn an_account_without_a_five_hour_limit_counts_as_having_all_the_room() {
        // an Ultra account reports weekly numbers only; it goes before an account at 12% of its 5-hour limit
        let profiles = [account("main", true), account("ultra", true), account("emir", true)];
        let mut usage = BTreeMap::from([("ultra".to_string(), state(None, Some(40.0))), ("emir".to_string(), state(Some(12.0), None))]);
        let best = suggest_account(&profiles, &usage, NOW, "main", None).unwrap();
        assert_eq!(best.profile_id, "ultra");
        assert_eq!(best.five_hour_pct, None);
        // without any numbers at all the account still comes last
        usage.remove("ultra");
        assert_eq!(suggest_account(&profiles, &usage, NOW, "main", None).unwrap().profile_id, "emir");
    }

    #[test]
    fn does_not_suggest_an_account_whose_limit_for_the_sessions_model_is_full() {
        let profiles = [account("main", true), account("elif", true), account("emir", true)];
        let usage = BTreeMap::from([
            ("elif".to_string(), UsageState { models: fable(100.0), ..state(Some(5.0), None) }),
            ("emir".to_string(), state(Some(60.0), None)),
        ]);
        let pick = |model: Option<&str>| suggest_account(&profiles, &usage, NOW, "main", model).map(|s| s.profile_id);
        assert_eq!(pick(Some("Fable 5 (1M context)")).as_deref(), Some("emir"));
        assert_eq!(pick(Some("Opus 5.5")).as_deref(), Some("elif"));
        assert_eq!(pick(None).as_deref(), Some("elif"));
    }

    #[test]
    fn works_on_references_as_well() {
        let profiles = [account("main", true), account("elif", true)];
        let refs: Vec<&SwitchProfile> = profiles.iter().collect();
        assert_eq!(suggest_account(&refs, &BTreeMap::new(), NOW, "main", None).map(|s| s.profile_id).as_deref(), Some("elif"));
    }
}
