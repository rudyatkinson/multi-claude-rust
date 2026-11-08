//! Local wall-clock time. `/usage` prints its reset times in the machine's time zone ("Oct 7,
//! 7:10pm"), and the Electron version read them with `new Date(year, month, day, hour, minute)`.
//! This crate has no time zone database, so the conversion is behind [`LocalClock`]: the app
//! supplies the machine's zone, the tests a fixed offset. The calendar arithmetic itself (proleptic
//! Gregorian, days since 1970-01-01) is here.

const MS_PER_MINUTE: i64 = 60_000;
const MINUTES_PER_DAY: i64 = 1440;

/// A date and time on a wall clock, without a zone. `month` is 1-12, `day` 1-31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CivilDateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

impl CivilDateTime {
    /// Takes its fields as `new Date(year, month0, day, hour, minute)` does: `month0` counts from
    /// 0, and a field that runs over (month 12, day 0 or 32, minute 75) carries into the next unit.
    pub fn normalized(year: i64, month0: i64, day: i64, hour: i64, minute: i64) -> Self {
        let year = year + month0.div_euclid(12);
        let month = (month0.rem_euclid(12) + 1) as u32;
        let days = days_from_civil(year, month, 1) + (day - 1);
        Self::from_minutes(days * MINUTES_PER_DAY + hour * 60 + minute)
    }

    /// The wall-clock time of UTC at `epoch_ms`.
    pub fn from_utc_ms(epoch_ms: i64) -> Self {
        Self::from_minutes(epoch_ms.div_euclid(MS_PER_MINUTE))
    }

    /// The instant at which a UTC clock shows this time.
    pub fn utc_ms(&self) -> i64 {
        let days = days_from_civil(i64::from(self.year), self.month, self.day);
        (days * MINUTES_PER_DAY + i64::from(self.hour) * 60 + i64::from(self.minute)) * MS_PER_MINUTE
    }

    /// The same wall-clock time `days` later, as `setDate(getDate() + days)` moves a date.
    pub fn plus_days(&self, days: i64) -> Self {
        Self::normalized(
            i64::from(self.year),
            i64::from(self.month) - 1,
            i64::from(self.day) + days,
            i64::from(self.hour),
            i64::from(self.minute),
        )
    }

    fn from_minutes(minutes: i64) -> Self {
        let days = minutes.div_euclid(MINUTES_PER_DAY);
        let rest = minutes.rem_euclid(MINUTES_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        CivilDateTime {
            year: year.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            month,
            day,
            hour: (rest / 60) as u32,
            minute: (rest % 60) as u32,
        }
    }
}

/// The machine's time zone, as far as reading and writing local times needs it.
pub trait LocalClock: Send + Sync {
    /// The local wall-clock time at the instant `epoch_ms`.
    fn local_at(&self, epoch_ms: i64) -> CivilDateTime;

    /// The instant at which the local clock shows `local`, as `new Date(...).getTime()` finds it: a
    /// time that a daylight saving change skips counts as the same time after the change (it moves
    /// forward by the gap), a time that occurs twice is the earlier instant.
    fn to_epoch_ms(&self, local: CivilDateTime) -> i64;
}

/// A zone that is always the same distance from UTC: UTC itself in the tests, or a stand-in for
/// the machine's zone where no time zone rules are at hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedOffsetClock {
    /// Local time minus UTC, in minutes (+180 for Istanbul).
    pub offset_minutes: i32,
}

impl FixedOffsetClock {
    pub const UTC: FixedOffsetClock = FixedOffsetClock { offset_minutes: 0 };
}

impl LocalClock for FixedOffsetClock {
    fn local_at(&self, epoch_ms: i64) -> CivilDateTime {
        CivilDateTime::from_utc_ms(epoch_ms + i64::from(self.offset_minutes) * MS_PER_MINUTE)
    }

    fn to_epoch_ms(&self, local: CivilDateTime) -> i64 {
        local.utc_ms() - i64::from(self.offset_minutes) * MS_PER_MINUTE
    }
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar (Howard Hinnant's algorithm).
/// `day` may lie outside the month; the result then lies before or after it.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let month = i64::from(month);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after 1970-01-01: (year, month 1-12, day 1-31).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> CivilDateTime {
        CivilDateTime { year, month, day, hour, minute }
    }

    #[test]
    fn counts_days_both_ways() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        for days in [-800_000, -1, 0, 59, 60, 11_016, 20_733, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn matches_known_instants() {
        // Date.UTC(2026, 9, 7, 15, 0) and Date.parse('2025-02-01T16:00:00Z')
        assert_eq!(at(2026, 10, 7, 15, 0).utc_ms(), 1_791_385_200_000);
        assert_eq!(at(2025, 2, 1, 16, 0).utc_ms(), 1_738_425_600_000);
        assert_eq!(CivilDateTime::from_utc_ms(1_738_425_600_000), at(2025, 2, 1, 16, 0));
        assert_eq!(CivilDateTime::from_utc_ms(-1), at(1969, 12, 31, 23, 59));
    }

    #[test]
    fn carries_fields_that_run_over_as_date_does() {
        // new Date(2026, 12, 1) is January 2027; day 0 is the last day of the month before
        assert_eq!(CivilDateTime::normalized(2026, 12, 1, 0, 0), at(2027, 1, 1, 0, 0));
        assert_eq!(CivilDateTime::normalized(2026, 2, 0, 0, 0), at(2026, 2, 28, 0, 0));
        assert_eq!(CivilDateTime::normalized(2024, 1, 30, 0, 0), at(2024, 3, 1, 0, 0));
        assert_eq!(CivilDateTime::normalized(2026, 9, 7, 23, 75), at(2026, 10, 8, 0, 15));
        assert_eq!(at(2026, 12, 31, 9, 0).plus_days(1), at(2027, 1, 1, 9, 0));
    }

    #[test]
    fn fixed_offsets_round_trip() {
        let istanbul = FixedOffsetClock { offset_minutes: 180 };
        let utc_noon = at(2026, 10, 7, 12, 0).utc_ms();
        assert_eq!(istanbul.local_at(utc_noon), at(2026, 10, 7, 15, 0));
        assert_eq!(istanbul.to_epoch_ms(at(2026, 10, 7, 15, 0)), utc_noon);
        assert_eq!(FixedOffsetClock::UTC.to_epoch_ms(at(2026, 10, 7, 12, 0)), utc_noon);
    }
}
