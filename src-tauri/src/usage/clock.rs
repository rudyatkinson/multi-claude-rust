//! The machine's time zone for reading `/usage` reset times, with its daylight saving rules (through
//! `chrono`'s `Local`): `local_at` is the wall-clock time at an instant, and `to_epoch_ms` reads a
//! wall-clock time the way `new Date(year, month, day, hour, minute)` did.

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use mc_core::usage::{CivilDateTime, LocalClock};

/// The machine's time zone as it is now; a change of the zone while the app runs is followed.
pub struct SystemClock;

fn civil(local: NaiveDateTime) -> CivilDateTime {
    CivilDateTime { year: local.year(), month: local.month(), day: local.day(), hour: local.hour(), minute: local.minute() }
}

/// The earlier of two instants a local time can stand for, when it stands for any.
fn earliest(found: LocalResult<DateTime<Local>>) -> Option<DateTime<Local>> {
    match found {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Some(at),
        LocalResult::None => None,
    }
}

impl LocalClock for SystemClock {
    fn local_at(&self, epoch_ms: i64) -> CivilDateTime {
        match earliest(Local.timestamp_millis_opt(epoch_ms)) {
            Some(at) => civil(at.naive_local()),
            // outside the range chrono can show
            None => CivilDateTime::from_utc_ms(epoch_ms),
        }
    }

    fn to_epoch_ms(&self, local: CivilDateTime) -> i64 {
        let Some(naive) = NaiveDate::from_ymd_opt(local.year, local.month, local.day).and_then(|d| d.and_hms_opt(local.hour, local.minute, 0))
        else {
            return local.utc_ms();
        };
        if let Some(at) = earliest(Local.from_local_datetime(&naive)) {
            return at.timestamp_millis();
        }
        // a time the clock skips (spring forward): read with the offset before the change, which
        // lands as far after the change as the time was after its start, as JavaScript does
        match earliest(Local.from_local_datetime(&(naive - Duration::hours(3)))) {
            Some(before) => naive.and_utc().timestamp_millis() - i64::from(before.offset().local_minus_utc()) * 1000,
            None => local.utc_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_round_trip() {
        let clock = SystemClock;
        let at = CivilDateTime { year: 2026, month: 10, day: 7, hour: 15, minute: 20 };
        assert_eq!(clock.local_at(clock.to_epoch_ms(at)), at);
        let now = chrono::Utc::now().timestamp_millis();
        let minute = now - now.rem_euclid(60_000);
        assert_eq!(clock.to_epoch_ms(clock.local_at(minute)), minute);
    }

    #[test]
    fn an_impossible_date_does_not_panic() {
        let clock = SystemClock;
        let at = CivilDateTime { year: 2026, month: 2, day: 31, hour: 25, minute: 0 };
        assert_eq!(clock.to_epoch_ms(at), at.utc_ms());
    }
}
