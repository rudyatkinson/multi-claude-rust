//! When the user hears about usage (`src/main/usage/alerts.ts`): a limit passing a threshold, and
//! a limit the user was warned about starting over.

use std::fmt;

use super::normalize::SAME_WINDOW_MS;
use crate::duration::countdown;
use crate::types::{NoticeLevel, UsageState, UsageWindow};

/// Usage percentages that are worth telling the user about, lowest first.
pub const ALERT_THRESHOLDS: [i64; 2] = [80, 95];

/// A reset that is noticed later than this (the computer slept) is old news.
const RESET_NOTICE_MAX_DELAY_MS: i64 = 30 * 60 * 1000;
/// Without reset times to compare, usage falling this far to below every threshold means a new window.
const NEW_WINDOW_DROP: f64 = 20.0;

/// A limit of an account: its 5-hour session, its week, or the week of one model.
/// Displayed as the TypeScript `WindowKey`: `fiveHour`, `sevenDay`, `model:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WindowKey {
    FiveHour,
    SevenDay,
    Model(String),
}

impl WindowKey {
    /// "5-hour limit", "weekly limit", "Fable weekly limit".
    pub fn label(&self) -> String {
        match self {
            WindowKey::FiveHour => "5-hour limit".to_string(),
            WindowKey::SevenDay => "weekly limit".to_string(),
            WindowKey::Model(name) => format!("{name} weekly limit"),
        }
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowKey::FiveHour => f.write_str("fiveHour"),
            WindowKey::SevenDay => f.write_str("sevenDay"),
            WindowKey::Model(name) => write!(f, "model:{name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlertEvent {
    Threshold {
        profile_id: String,
        window: WindowKey,
        /// Whole percent.
        pct: i64,
        threshold: i64,
        resets_at: Option<i64>,
    },
    Reset {
        profile_id: String,
        window: WindowKey,
    },
}

impl AlertEvent {
    pub fn profile_id(&self) -> &str {
        match self {
            AlertEvent::Threshold { profile_id, .. } | AlertEvent::Reset { profile_id, .. } => profile_id,
        }
    }

    pub fn window(&self) -> &WindowKey {
        match self {
            AlertEvent::Threshold { window, .. } | AlertEvent::Reset { window, .. } => window,
        }
    }
}

/// What [`LimitAlerts::tick`] found: limits that started over, and the accounts whose usage should
/// be fetched again because of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickResult {
    pub events: Vec<AlertEvent>,
    pub refresh: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Tracked {
    resets_at: Option<i64>,
    pct: f64,
    /// How many thresholds this window has passed, as far as the user knows.
    level: usize,
}

fn level_of(pct: f64) -> usize {
    ALERT_THRESHOLDS.iter().filter(|&&threshold| pct >= threshold as f64).count()
}

fn windows_of(state: &UsageState) -> Vec<(WindowKey, UsageWindow)> {
    let mut windows = Vec::new();
    if let Some(w) = &state.five_hour {
        windows.push((WindowKey::FiveHour, w.clone()));
    }
    if let Some(w) = &state.seven_day {
        windows.push((WindowKey::SevenDay, w.clone()));
    }
    for model in state.models.iter().flatten() {
        windows.push((WindowKey::Model(model.name.clone()), model.window()));
    }
    windows
}

/// The reset time identifies a window. `/usage` rounds it to the minute and the status line
/// reports it to the second, so the two disagree a little about the same window.
fn is_new_window(prev: &Tracked, next: &UsageWindow, level: usize) -> bool {
    if let (Some(prev_reset), Some(next_reset)) = (prev.resets_at, next.resets_at) {
        return (next_reset - prev_reset).abs() > SAME_WINDOW_MS;
    }
    level == 0 && next.used_pct <= prev.pct - NEW_WINDOW_DROP
}

/// The windows of one account, in the order they were first seen (a JavaScript `Map`).
type Known = Vec<(WindowKey, Tracked)>;

fn set(known: &mut Known, key: WindowKey, tracked: Tracked) {
    match known.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = tracked,
        None => known.push((key, tracked)),
    }
}

/// Turns the stream of usage numbers into the few moments worth a notification: a limit passing
/// a threshold, and a limit the user was warned about starting over.
///
/// The first numbers seen for a window only set the baseline. So nothing is announced when the
/// app starts, and nothing for usage that built up elsewhere before it was looked at.
#[derive(Debug, Clone, Default)]
pub struct LimitAlerts {
    /// Per account, in the order the accounts were first seen.
    tracked: Vec<(String, Known)>,
}

impl LimitAlerts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the current usage of one account and returns what is new about it.
    pub fn observe(&mut self, profile_id: &str, state: &UsageState, now: i64) -> Vec<AlertEvent> {
        let windows = windows_of(state);
        let known = match self.tracked.iter().position(|(id, _)| id == profile_id) {
            Some(i) => &mut self.tracked[i].1,
            None => {
                self.tracked.push((profile_id.to_string(), Vec::new()));
                &mut self.tracked.last_mut().expect("just pushed").1
            }
        };
        // a model limit that is no longer reported is forgotten
        known.retain(|(key, _)| !matches!(key, WindowKey::Model(_)) || windows.iter().any(|(k, _)| k == key));

        let mut events = Vec::new();
        for (key, win) in windows {
            // an idle session keeps re-sending the numbers of a window that is over
            if win.resets_at.is_some_and(|at| at <= now) {
                continue;
            }
            let level = level_of(win.used_pct);
            let prev = known.iter().find(|(k, _)| *k == key).map(|(_, t)| t.clone());
            let Some(prev) = prev.filter(|prev| !is_new_window(prev, &win, level)) else {
                // a window the user was warned about gave way to a fresh one before the clock noticed
                if let Some(prev) = known.iter().find(|(k, _)| *k == key).map(|(_, t)| t) {
                    let on_time = prev.resets_at.is_none_or(|at| now - at <= RESET_NOTICE_MAX_DELAY_MS);
                    if prev.level > 0 && level == 0 && on_time {
                        events.push(AlertEvent::Reset { profile_id: profile_id.to_string(), window: key.clone() });
                    }
                }
                set(known, key, Tracked { resets_at: win.resets_at, pct: win.used_pct, level });
                continue;
            };
            let resets_at = win.resets_at.or(prev.resets_at);
            if level > prev.level {
                events.push(AlertEvent::Threshold {
                    profile_id: profile_id.to_string(),
                    window: key.clone(),
                    pct: super::js_round(win.used_pct) as i64,
                    threshold: ALERT_THRESHOLDS[level - 1],
                    resets_at,
                });
            }
            set(known, key, Tracked { resets_at, pct: win.used_pct, level: level.max(prev.level) });
        }
        events
    }

    /// Called on a timer: reports the limits whose reset time has passed, and the accounts whose
    /// usage should be fetched again because of it.
    pub fn tick(&mut self, now: i64) -> TickResult {
        let mut result = TickResult::default();
        for (profile_id, known) in &mut self.tracked {
            let mut kept = Vec::with_capacity(known.len());
            for (key, window) in known.drain(..) {
                let Some(resets_at) = window.resets_at.filter(|&at| now >= at) else {
                    kept.push((key, window));
                    continue;
                };
                // whatever is reported next belongs to a new window
                if window.level == 0 {
                    continue;
                }
                if !result.refresh.contains(profile_id) {
                    result.refresh.push(profile_id.clone());
                }
                if now - resets_at <= RESET_NOTICE_MAX_DELAY_MS {
                    result.events.push(AlertEvent::Reset { profile_id: profile_id.clone(), window: key });
                }
            }
            *known = kept;
        }
        result
    }

    pub fn forget(&mut self, profile_id: &str) {
        self.tracked.retain(|(id, _)| id != profile_id);
    }
}

/// The words for what happened to one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertMessage {
    pub title: String,
    /// May be empty.
    pub body: String,
    /// `Warn` when a threshold was passed, `Info` for good news only.
    pub level: NoticeLevel,
}

fn sentence(event: &AlertEvent) -> String {
    match event {
        AlertEvent::Reset { window, .. } => format!("{} has reset", window.label()),
        AlertEvent::Threshold { window, pct, .. } => format!("{} at {pct}%", window.label()),
    }
}

/// `text.charAt(0).toUpperCase() + text.slice(1)`: a first character outside the BMP is half a
/// surrogate pair to `charAt` and stays as it is.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.len_utf16() == 1 => first.to_uppercase().chain(chars).collect(),
        _ => text.to_string(),
    }
}

/// The words for what happened to one account: the most pressing event in the title, the rest
/// below. None for no events.
pub fn alert_message(profile_name: &str, events: &[AlertEvent], now: i64) -> Option<AlertMessage> {
    // highest threshold first, resets last
    let rank = |e: &AlertEvent| match e {
        AlertEvent::Threshold { threshold, .. } => -threshold,
        AlertEvent::Reset { .. } => 0,
    };
    let mut sorted: Vec<&AlertEvent> = events.iter().collect();
    sorted.sort_by_key(|e| rank(e));
    let (first, rest) = sorted.split_first()?;
    let mut lines: Vec<String> = Vec::with_capacity(events.len());
    match first {
        AlertEvent::Threshold { resets_at: Some(at), .. } => lines.push(format!("Resets in {}.", countdown(at - now))),
        AlertEvent::Reset { .. } => lines.push("The account can be used again.".to_string()),
        AlertEvent::Threshold { .. } => {}
    }
    lines.extend(rest.iter().map(|e| format!("{}.", capitalized(&sentence(e)))));
    let warn = events.iter().any(|e| matches!(e, AlertEvent::Threshold { .. }));
    Some(AlertMessage {
        title: format!("{profile_name}: {}", sentence(first)),
        body: lines.join(" "),
        level: if warn { NoticeLevel::Warn } else { NoticeLevel::Info },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelUsage;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    // Date.UTC(2026, 9, 7, 12, 0, 0)
    const T0: i64 = 1_791_374_400_000;

    /// Usage of a 5-hour window; by default the one that resets three hours after T0.
    fn five_at(used_pct: f64, resets_at: i64) -> UsageState {
        UsageState { five_hour: Some(UsageWindow { used_pct, resets_at: Some(resets_at) }), ..UsageState::default() }
    }

    fn five(used_pct: f64) -> UsageState {
        five_at(used_pct, T0 + 3 * HOUR)
    }

    /// The same without a reset time, as `/usage` reports it for a window that has not started.
    fn bare(used_pct: f64) -> UsageState {
        UsageState { five_hour: Some(UsageWindow { used_pct, resets_at: None }), ..UsageState::default() }
    }

    fn kinds(events: &[AlertEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                AlertEvent::Threshold { window, threshold, .. } => format!("{window}@{threshold}"),
                AlertEvent::Reset { window, .. } => format!("{window} reset"),
            })
            .collect()
    }

    fn reset(profile_id: &str, window: WindowKey) -> AlertEvent {
        AlertEvent::Reset { profile_id: profile_id.into(), window }
    }

    #[test]
    fn says_nothing_about_the_first_numbers_it_sees_however_high() {
        let mut alerts = LimitAlerts::new();
        assert_eq!(alerts.observe("a", &five(97.0), T0), vec![]);
        // and nothing again for the same level
        assert_eq!(alerts.observe("a", &five(98.0), T0 + MIN), vec![]);
    }

    #[test]
    fn alerts_once_when_a_threshold_is_passed() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(40.0), T0);
        assert_eq!(alerts.observe("a", &five(79.0), T0 + MIN), vec![]);
        assert_eq!(
            alerts.observe("a", &five(81.4), T0 + 2 * MIN),
            vec![AlertEvent::Threshold {
                profile_id: "a".into(),
                window: WindowKey::FiveHour,
                pct: 81,
                threshold: 80,
                resets_at: Some(T0 + 3 * HOUR)
            }]
        );
        assert_eq!(alerts.observe("a", &five(88.0), T0 + 3 * MIN), vec![]);
        assert_eq!(kinds(&alerts.observe("a", &five(95.0), T0 + 4 * MIN)), ["fiveHour@95"]);
        assert_eq!(alerts.observe("a", &five(100.0), T0 + 5 * MIN), vec![]);
    }

    #[test]
    fn reports_only_the_highest_threshold_when_usage_jumps_past_both() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(60.0), T0);
        assert_eq!(kinds(&alerts.observe("a", &five(96.0), T0 + MIN)), ["fiveHour@95"]);
    }

    #[test]
    fn does_not_alert_again_when_a_lower_reading_follows() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(60.0), T0);
        alerts.observe("a", &five(82.0), T0 + MIN);
        // `/usage` and the status line round differently
        assert_eq!(alerts.observe("a", &five(79.0), T0 + 2 * MIN), vec![]);
        assert_eq!(alerts.observe("a", &five(83.0), T0 + 3 * MIN), vec![]);
    }

    #[test]
    fn treats_reset_times_a_little_apart_as_the_same_window() {
        let mut alerts = LimitAlerts::new();
        let at = T0 + 3 * HOUR;
        alerts.observe("a", &five_at(70.0, at), T0);
        // the status line reports seconds, `/usage` whole minutes
        assert_eq!(kinds(&alerts.observe("a", &five_at(85.0, at + 40_000), T0 + MIN)), ["fiveHour@80"]);
        assert_eq!(alerts.observe("a", &five_at(86.0, at - 20_000), T0 + 2 * MIN), vec![]);
        assert_eq!(alerts.observe("a", &five_at(87.0, at + 40_000), T0 + 3 * MIN), vec![]);
    }

    #[test]
    fn starts_over_with_a_new_window_whose_first_numbers_are_a_baseline_again() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(50.0), T0);
        alerts.observe("a", &five(96.0), T0 + MIN);
        let next = T0 + 8 * HOUR;
        // the new window is already used heavily when it is first seen: no alert for that
        assert_eq!(alerts.observe("a", &five_at(85.0, next), T0 + 4 * HOUR), vec![]);
        assert_eq!(kinds(&alerts.observe("a", &five_at(95.0, next), T0 + 5 * HOUR)), ["fiveHour@95"]);
    }

    #[test]
    fn announces_the_reset_when_a_warned_about_window_gives_way_to_a_fresh_one() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(50.0), T0);
        alerts.observe("a", &five(96.0), T0 + MIN);
        assert_eq!(kinds(&alerts.observe("a", &five_at(2.0, T0 + 8 * HOUR), T0 + 3 * HOUR + MIN)), ["fiveHour reset"]);
        // a window nobody was warned about ends quietly
        alerts.observe("b", &five(30.0), T0);
        assert_eq!(alerts.observe("b", &five_at(1.0, T0 + 8 * HOUR), T0 + 3 * HOUR + MIN), vec![]);
    }

    #[test]
    fn is_not_fooled_by_a_stale_reading_right_at_the_reset_time() {
        let mut alerts = LimitAlerts::new();
        let at = T0 + 3 * HOUR;
        alerts.observe("a", &five_at(50.0, at), T0);
        alerts.observe("a", &five_at(96.0, at), T0 + MIN);
        // the old percentage with a reset time a day ahead
        assert_eq!(alerts.observe("a", &five_at(96.0, at + 24 * HOUR), at + MIN), vec![]);
        // then the real new window
        assert_eq!(kinds(&alerts.observe("a", &five_at(3.0, at + 5 * HOUR), at + 3 * MIN)), ["fiveHour reset"]);
        assert_eq!(alerts.observe("a", &five_at(10.0, at + 5 * HOUR), at + 10 * MIN), vec![]);
    }

    #[test]
    fn ignores_numbers_of_a_window_that_is_already_over() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(50.0), T0);
        // an idle session re-sends its last response
        assert_eq!(alerts.observe("a", &five_at(99.0, T0 - HOUR), T0 + MIN), vec![]);
        assert_eq!(kinds(&alerts.observe("a", &five(81.0), T0 + 2 * MIN)), ["fiveHour@80"]);
    }

    #[test]
    fn without_reset_times_takes_a_large_drop_as_a_new_window() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &bare(60.0), T0);
        assert_eq!(kinds(&alerts.observe("a", &bare(90.0), T0 + MIN)), ["fiveHour@80"]);
        // a small drop is noise
        assert_eq!(alerts.observe("a", &bare(85.0), T0 + 2 * MIN), vec![]);
        assert_eq!(kinds(&alerts.observe("a", &bare(5.0), T0 + 6 * HOUR)), ["fiveHour reset"]);
        assert_eq!(kinds(&alerts.observe("a", &bare(81.0), T0 + 7 * HOUR)), ["fiveHour@80"]);
    }

    #[test]
    fn watches_the_week_and_each_model_limit_on_their_own_per_account() {
        let mut alerts = LimitAlerts::new();
        let week = T0 + 3 * 24 * HOUR;
        let state = |five_hour: f64, seven_day: f64, fable: f64| UsageState {
            five_hour: Some(UsageWindow { used_pct: five_hour, resets_at: Some(T0 + 3 * HOUR) }),
            seven_day: Some(UsageWindow { used_pct: seven_day, resets_at: Some(week) }),
            models: Some(vec![ModelUsage { name: "Fable".into(), used_pct: fable, resets_at: Some(week) }]),
            observed_at: None,
        };
        alerts.observe("a", &state(10.0, 70.0, 70.0), T0);
        alerts.observe("b", &state(10.0, 70.0, 70.0), T0);
        assert_eq!(kinds(&alerts.observe("a", &state(10.0, 82.0, 96.0), T0 + MIN)), ["sevenDay@80", "model:Fable@95"]);
        // the other account is untouched
        assert_eq!(alerts.observe("b", &state(10.0, 71.0, 71.0), T0 + MIN), vec![]);
    }

    #[test]
    fn forgets_a_model_limit_that_is_no_longer_reported() {
        let mut alerts = LimitAlerts::new();
        let fable = |used_pct: f64| UsageState {
            models: Some(vec![ModelUsage { name: "Fable".into(), used_pct, resets_at: Some(T0 + 24 * HOUR) }]),
            ..UsageState::default()
        };
        alerts.observe("a", &fable(70.0), T0);
        alerts.observe("a", &UsageState::default(), T0 + MIN);
        // back again: a first look, not a crossing
        assert_eq!(alerts.observe("a", &fable(90.0), T0 + 2 * MIN), vec![]);
    }

    // ---------- tick ----------

    #[test]
    fn announces_the_reset_of_a_limit_the_user_was_warned_about_once() {
        let mut alerts = LimitAlerts::new();
        let at = T0 + 3 * HOUR;
        alerts.observe("a", &five_at(50.0, at), T0);
        alerts.observe("a", &five_at(85.0, at), T0 + MIN);
        assert_eq!(alerts.tick(at - MIN), TickResult::default());
        assert_eq!(alerts.tick(at + 10_000), TickResult { events: vec![reset("a", WindowKey::FiveHour)], refresh: vec!["a".into()] });
        assert_eq!(alerts.tick(at + 40_000), TickResult::default());
        // the numbers that follow are a new baseline
        assert_eq!(alerts.observe("a", &five_at(90.0, at + 5 * HOUR), at + MIN), vec![]);
    }

    #[test]
    fn also_counts_a_limit_that_was_already_high_when_first_seen() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(97.0), T0);
        assert_eq!(kinds(&alerts.tick(T0 + 3 * HOUR).events), ["fiveHour reset"]);
    }

    #[test]
    fn keeps_quiet_about_limits_that_never_got_high() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(40.0), T0);
        assert_eq!(alerts.tick(T0 + 3 * HOUR), TickResult::default());
    }

    #[test]
    fn skips_a_reset_that_is_noticed_much_too_late_but_still_refreshes() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(97.0), T0);
        // the computer slept through it
        assert_eq!(alerts.tick(T0 + 3 * HOUR + 31 * MIN), TickResult { events: vec![], refresh: vec!["a".into()] });
    }

    #[test]
    fn forgets_an_account() {
        let mut alerts = LimitAlerts::new();
        alerts.observe("a", &five(97.0), T0);
        alerts.forget("a");
        assert_eq!(alerts.tick(T0 + 3 * HOUR), TickResult::default());
    }

    #[test]
    fn lists_accounts_to_refresh_once_in_the_order_first_seen() {
        let mut alerts = LimitAlerts::new();
        let both = UsageState { seven_day: Some(UsageWindow { used_pct: 90.0, resets_at: Some(T0 + 3 * HOUR) }), ..five(97.0) };
        alerts.observe("b", &both, T0);
        alerts.observe("a", &five(97.0), T0);
        let result = alerts.tick(T0 + 3 * HOUR);
        assert_eq!(result.refresh, ["b", "a"]);
        assert_eq!(kinds(&result.events), ["fiveHour reset", "sevenDay reset", "fiveHour reset"]);
    }

    // ---------- alertMessage ----------

    #[test]
    fn names_the_account_the_limit_and_the_time_left() {
        let event = AlertEvent::Threshold {
            profile_id: "a".into(),
            window: WindowKey::FiveHour,
            pct: 82,
            threshold: 80,
            resets_at: Some(T0 + 80 * MIN),
        };
        assert_eq!(
            alert_message("Elif", &[event], T0),
            Some(AlertMessage { title: "Elif: 5-hour limit at 82%".into(), body: "Resets in 1h 20m.".into(), level: NoticeLevel::Warn })
        );
    }

    #[test]
    fn puts_the_most_pressing_limit_first_and_the_rest_below() {
        let events = [
            AlertEvent::Threshold {
                profile_id: "a".into(),
                window: WindowKey::SevenDay,
                pct: 81,
                threshold: 80,
                resets_at: Some(T0 + 50 * HOUR),
            },
            AlertEvent::Threshold {
                profile_id: "a".into(),
                window: WindowKey::Model("Fable".into()),
                pct: 96,
                threshold: 95,
                resets_at: Some(T0 + 50 * HOUR),
            },
        ];
        assert_eq!(
            alert_message("Main", &events, T0),
            Some(AlertMessage {
                title: "Main: Fable weekly limit at 96%".into(),
                body: "Resets in 2d 2h. Weekly limit at 81%.".into(),
                level: NoticeLevel::Warn
            })
        );
    }

    #[test]
    fn tells_of_a_reset_as_good_news() {
        assert_eq!(
            alert_message("Emir", &[reset("a", WindowKey::FiveHour)], T0),
            Some(AlertMessage {
                title: "Emir: 5-hour limit has reset".into(),
                body: "The account can be used again.".into(),
                level: NoticeLevel::Info
            })
        );
    }

    #[test]
    fn mixes_news_and_says_nothing_for_no_events() {
        let events = [
            reset("a", WindowKey::SevenDay),
            AlertEvent::Threshold { profile_id: "a".into(), window: WindowKey::FiveHour, pct: 80, threshold: 80, resets_at: None },
        ];
        assert_eq!(
            alert_message("Work", &events, T0),
            Some(AlertMessage { title: "Work: 5-hour limit at 80%".into(), body: "Weekly limit has reset.".into(), level: NoticeLevel::Warn })
        );
        assert_eq!(alert_message("Work", &[], T0), None);
    }
}
