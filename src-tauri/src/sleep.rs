//! Puts idle Claude tabs to sleep. Every [`SLEEP_CHECK_MS`] the sessions are looked at, and those
//! that have been quiet for `sleepIdleMinutes` (see `mc_core::sleep` for the rule) end their
//! process and stay in their tab as sleeping ones (`app::launch::sleep_session`). They wake when
//! the user clicks into them or types (`sessions:wake`).
//!
//! What the check reads of the rest of the app, and what it does, comes in through [`SleepDeps`],
//! so that one round ([`tick`]) can be tried with a fake app and a clock of its own.

use std::time::Duration;

use mc_core::sleep::{should_sleep, SleepInput, SLEEP_CHECK_MS};
use tauri::AppHandle;

use crate::app::{now_ms, state as app_state};
use crate::sessions::Activity;
use crate::state::{is, lock};

/// What the idle check needs of the app.
pub trait SleepDeps: Send + Sync {
    /// `sleepIdleMinutes` as it is used; 0 switches the check off.
    fn after_minutes(&self) -> u32;
    /// Every session, with when it was last typed into and when it last printed something.
    fn sessions(&self) -> Vec<Activity>;
    /// The session the user is looking at (`ui:visible`).
    fn watched(&self) -> Option<String>;
    /// A workflow run waits for the tab.
    fn is_workflow_step(&self, id: &str) -> bool;
    /// Puts a session to sleep. Returns at once: its process ends in the background.
    fn put_to_sleep(&self, id: &str);
}

/// One round of the check at `now` (ms since the epoch): every session that is idle by the rule
/// is put to sleep. Returns their ids.
pub fn tick(deps: &dyn SleepDeps, now: i64) -> Vec<String> {
    let after_minutes = deps.after_minutes();
    if after_minutes == 0 {
        return Vec::new();
    }
    let watched = deps.watched();
    let idle: Vec<String> = deps
        .sessions()
        .into_iter()
        .filter(|activity| {
            let info = &activity.info;
            let input = SleepInput {
                kind: info.kind,
                status: info.status,
                turn: info.turn.as_ref(),
                last_input_at: activity.last_input_at,
                last_output_at: activity.last_output_at,
                moving: info.moving,
                keep_awake: info.keep_awake,
                workflow_step: deps.is_workflow_step(&info.id),
                watched: watched.as_deref() == Some(info.id.as_str()),
                after_minutes,
            };
            should_sleep(input, now)
        })
        .map(|activity| activity.info.id)
        .collect();
    for id in &idle {
        deps.put_to_sleep(id);
    }
    idle
}

/// The app as the check sees it.
struct AppSleep(AppHandle);

impl SleepDeps for AppSleep {
    fn after_minutes(&self) -> u32 {
        app_state(&self.0).map_or(0, |state| state.settings().sleep_after_minutes())
    }

    fn sessions(&self) -> Vec<Activity> {
        app_state(&self.0).map(|state| state.sessions.activity()).unwrap_or_default()
    }

    fn watched(&self) -> Option<String> {
        app_state(&self.0).and_then(|state| lock(&state.visible).clone())
    }

    fn is_workflow_step(&self, id: &str) -> bool {
        app_state(&self.0).is_some_and(|state| state.runs.watches(id))
    }

    fn put_to_sleep(&self, id: &str) {
        let handle = self.0.clone();
        let id = id.to_owned();
        tauri::async_runtime::spawn(async move {
            let Some(state) = app_state(&handle) else { return };
            if let Err(e) = crate::app::launch::sleep_session(&state, &id).await {
                // closed, woken or moved since the check looked at it
                log::debug!("putting a tab to sleep: {e}");
            }
        });
    }
}

/// Starts the check; it runs until the app quits. Quitting ends the running sessions as before:
/// nothing is put to sleep on the way out.
pub fn start(app: &AppHandle) {
    let deps = AppSleep(app.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(SLEEP_CHECK_MS)).await;
            if app_state(&deps.0).is_none_or(|state| is(&state.flags.quitting)) {
                return;
            }
            let slept = tick(&deps, now_ms());
            if !slept.is_empty() {
                log::info!("putting {} idle tab(s) to sleep", slept.len());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use mc_core::types::{SessionInfo, SessionKind, SessionStatus, Turn, TurnKind};

    use super::*;

    const MIN: i64 = 60_000;
    const NOW: i64 = 100 * MIN;

    fn info(id: &str) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            profile_id: "work".into(),
            kind: SessionKind::Claude,
            cwd: "/proj".into(),
            title: "proj".into(),
            status: SessionStatus::Running,
            exit_code: None,
            started_at: 0,
            name: None,
            replaces: None,
            place: None,
            claude_session_id: None,
            model: None,
            context_pct: None,
            turn: None,
            moving: false,
            arrived: None,
            keep_awake: false,
            slept_at: None,
        }
    }

    /// A session last typed into and last printing at these minutes before [`NOW`].
    fn quiet(info: SessionInfo, input_ago: i64, output_ago: i64) -> Activity {
        Activity { info, last_input_at: NOW - input_ago * MIN, last_output_at: NOW - output_ago * MIN }
    }

    struct Fake {
        minutes: u32,
        sessions: Vec<Activity>,
        watched: Option<String>,
        steps: HashSet<String>,
        slept: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(sessions: Vec<Activity>) -> Self {
            Fake { minutes: 15, sessions, watched: None, steps: HashSet::new(), slept: Mutex::default() }
        }
        fn slept(&self) -> Vec<String> {
            lock(&self.slept).clone()
        }
    }

    impl SleepDeps for Fake {
        fn after_minutes(&self) -> u32 {
            self.minutes
        }
        fn sessions(&self) -> Vec<Activity> {
            self.sessions.clone()
        }
        fn watched(&self) -> Option<String> {
            self.watched.clone()
        }
        fn is_workflow_step(&self, id: &str) -> bool {
            self.steps.contains(id)
        }
        fn put_to_sleep(&self, id: &str) {
            lock(&self.slept).push(id.to_owned());
        }
    }

    #[test]
    fn puts_the_quiet_tabs_to_sleep_and_leaves_the_others() {
        let done = |at_ago: i64| Some(Turn { at: NOW - at_ago * MIN, kind: TurnKind::Done { background: false } });
        let mut login = info("login");
        login.kind = SessionKind::Login;
        let mut working = info("working");
        working.turn = Some(Turn { at: 0, kind: TurnKind::Working });
        let mut finished = info("finished");
        finished.turn = done(30);
        let mut kept = info("kept");
        kept.keep_awake = true;
        let mut moving = info("moving");
        moving.moving = true;
        let mut asleep = info("asleep");
        asleep.status = SessionStatus::Sleeping;
        let fake = Fake {
            watched: Some("looked-at".into()),
            steps: HashSet::from(["step".to_owned()]),
            ..Fake::new(vec![
                quiet(info("idle"), 20, 20),
                quiet(info("typed"), 5, 20),
                quiet(info("printing"), 20, 1),
                // its turn ended half an hour ago; the redraw a minute ago does not count
                quiet(finished, 40, 1),
                quiet(info("looked-at"), 20, 20),
                quiet(info("step"), 20, 20),
                quiet(login, 20, 20),
                quiet(working, 20, 20),
                quiet(kept, 20, 20),
                quiet(moving, 20, 20),
                quiet(asleep, 20, 20),
            ])
        };
        assert_eq!(tick(&fake, NOW), ["idle", "finished"]);
        assert_eq!(fake.slept(), ["idle", "finished"]);
    }

    #[test]
    fn follows_the_setting_and_the_clock() {
        let sessions = vec![quiet(info("a"), 3, 3)];
        // switched off: nothing is even looked at
        let off = Fake { minutes: 0, ..Fake::new(sessions.clone()) };
        assert!(tick(&off, NOW).is_empty());
        assert!(off.slept().is_empty());
        // fifteen minutes: three are not enough, until the clock has moved on
        let fake = Fake::new(sessions.clone());
        assert!(tick(&fake, NOW).is_empty());
        assert_eq!(tick(&fake, NOW + 12 * MIN), ["a"]);
        // one minute
        let short = Fake { minutes: 1, ..Fake::new(sessions) };
        assert_eq!(tick(&short, NOW), ["a"]);
    }
}
