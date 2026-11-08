//! The machine's time zone for reading `/usage` reset times, until a real time zone library is
//! wired in with `UsageTracker::set_clock`. On Windows it is the current offset of the system
//! zone (`ActiveTimeBias`, which follows daylight saving time as it is now, not as it will be at
//! the reset time); elsewhere UTC.

use mc_core::usage::{CivilDateTime, FixedOffsetClock, LocalClock};

pub(crate) struct SystemClock;

impl SystemClock {
    fn now_offset() -> FixedOffsetClock {
        FixedOffsetClock { offset_minutes: offset_minutes().unwrap_or(0) }
    }
}

impl LocalClock for SystemClock {
    fn local_at(&self, epoch_ms: i64) -> CivilDateTime {
        Self::now_offset().local_at(epoch_ms)
    }

    fn to_epoch_ms(&self, local: CivilDateTime) -> i64 {
        Self::now_offset().to_epoch_ms(local)
    }
}

/// Local time minus UTC, in minutes.
#[cfg(windows)]
fn offset_minutes() -> Option<i32> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation").ok()?;
    // UTC = local time + bias; the DWORD holds a signed number
    let bias = key.get_value::<u32, _>("ActiveTimeBias").ok()? as i32;
    Some(-bias)
}

#[cfg(not(windows))]
fn offset_minutes() -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_is_plausible() {
        let offset = offset_minutes().unwrap_or(0);
        assert!((-14 * 60..=14 * 60).contains(&offset), "{offset}");
        let clock = SystemClock;
        let at = CivilDateTime { year: 2026, month: 10, day: 7, hour: 15, minute: 20 };
        assert_eq!(clock.local_at(clock.to_epoch_ms(at)), at);
    }
}
