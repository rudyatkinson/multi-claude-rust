//! Tray menu lines and tooltip (`src/main/trayModel.ts`).

use std::collections::BTreeMap;

use crate::types::{Profile, ProfileView, UsageState};
use crate::usage::usage_view;

/// As much of an account as a tray line needs: `{ id, name, account?: { loggedIn } }`.
pub trait TrayAccount {
    fn tray_id(&self) -> &str;
    fn tray_name(&self) -> &str;
    /// Its status is known and says it is logged out (`account && !account.loggedIn`). An account
    /// whose status is not known yet, or a token account, is not.
    fn logged_out(&self) -> bool;
}

impl TrayAccount for Profile {
    fn tray_id(&self) -> &str {
        &self.id
    }
    fn tray_name(&self) -> &str {
        &self.name
    }
    fn logged_out(&self) -> bool {
        self.account.as_ref().is_some_and(|a| !a.logged_in)
    }
}

impl TrayAccount for ProfileView {
    fn tray_id(&self) -> &str {
        &self.id
    }
    fn tray_name(&self) -> &str {
        &self.name
    }
    fn logged_out(&self) -> bool {
        self.account.as_ref().is_some_and(|a| !a.logged_in)
    }
}

impl<T: TrayAccount + ?Sized> TrayAccount for &T {
    fn tray_id(&self) -> &str {
        (**self).tray_id()
    }
    fn tray_name(&self) -> &str {
        (**self).tray_name()
    }
    fn logged_out(&self) -> bool {
        (**self).logged_out()
    }
}

/// One line per account for the tray menu: "Elif — 5h 42% · Week 61% · Fable 35%",
/// "Old — logged out", "Token — no usage data". A window whose reset time has passed shows 0%.
pub fn tray_account_lines<P: TrayAccount>(profiles: &[P], usage: &BTreeMap<String, UsageState>, now: i64) -> Vec<String> {
    profiles
        .iter()
        .map(|profile| {
            if profile.logged_out() {
                return format!("{} — logged out", profile.tray_name());
            }
            let mut parts = Vec::new();
            if let Some(state) = usage.get(profile.tray_id()) {
                if let Some(w) = &state.five_hour {
                    parts.push(format!("5h {}%", usage_view(w, now).pct));
                }
                if let Some(w) = &state.seven_day {
                    parts.push(format!("Week {}%", usage_view(w, now).pct));
                }
                for model in state.models.iter().flatten() {
                    parts.push(format!("{} {}%", model.name, usage_view(&model.window(), now).pct));
                }
            }
            let shown = if parts.is_empty() { "no usage data".to_string() } else { parts.join(" · ") };
            format!("{} — {shown}", profile.tray_name())
        })
        .collect()
}

/// "Multi Claude", with the number of running sessions when there are any.
pub fn tray_tooltip(running: usize) -> String {
    match running {
        0 => "Multi Claude".to_string(),
        1 => "Multi Claude — 1 session running".to_string(),
        n => format!("Multi Claude — {n} sessions running"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{ModelUsage, UsageWindow};

    // Date.UTC(2026, 9, 7, 12)
    const NOW: i64 = 1_791_374_400_000;
    const LATER: i64 = NOW + 3_600_000;

    fn profile(id: &str, name: &str, extra: serde_json::Value) -> Profile {
        let mut raw = json!({
            "id": id, "name": name, "slug": id, "color": "#d97757", "isDefault": false, "configDir": null,
            "share": { "dirs": [], "files": [], "localSettingsKeys": [] }
        });
        raw.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(raw).unwrap()
    }

    fn window(used_pct: f64, resets_at: Option<i64>) -> Option<UsageWindow> {
        Some(UsageWindow { used_pct, resets_at })
    }

    // ---------- tray menu (test/trayAndTransfer.test.ts) ----------

    #[test]
    fn shows_one_line_per_account_with_its_limits() {
        let account = |logged_in: bool| json!({ "account": { "loggedIn": logged_in, "checkedAt": NOW } });
        let profiles = [
            profile("main", "Main", account(true)),
            profile("e", "Elif", account(true)),
            profile("x", "Old", account(false)),
            profile("t", "Token", json!({ "auth": "token" })),
        ];
        let usage = BTreeMap::from([
            (
                "main".to_string(),
                UsageState {
                    five_hour: window(41.6, Some(LATER)),
                    seven_day: window(61.0, Some(LATER)),
                    models: Some(vec![ModelUsage { name: "Fable".into(), used_pct: 35.0, resets_at: Some(LATER) }]),
                    observed_at: None,
                },
            ),
            // the 5-hour window is over: it counts as empty until new numbers arrive
            ("e".to_string(), UsageState { five_hour: window(97.0, Some(NOW - 1000)), seven_day: window(12.0, None), ..UsageState::default() }),
        ]);
        assert_eq!(
            tray_account_lines(&profiles, &usage, NOW),
            ["Main — 5h 42% · Week 61% · Fable 35%", "Elif — 5h 0% · Week 12%", "Old — logged out", "Token — no usage data"]
        );
        // the views the renderer gets read the same
        let views: Vec<ProfileView> = profiles.iter().cloned().map(ProfileView::from).collect();
        assert_eq!(tray_account_lines(&views, &usage, NOW), tray_account_lines(&profiles, &usage, NOW));
    }

    #[test]
    fn counts_the_running_sessions_in_the_tooltip() {
        assert_eq!(tray_tooltip(0), "Multi Claude");
        assert_eq!(tray_tooltip(1), "Multi Claude — 1 session running");
        assert_eq!(tray_tooltip(3), "Multi Claude — 3 sessions running");
    }
}
