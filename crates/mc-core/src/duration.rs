//! `countdown` (`src/shared/duration.ts`) and `ago` (`src/renderer/src/format.ts`).

const MINUTE_MS: i64 = 60_000;

/// Time left, in the two largest units: "3d 8h", "2h 46m", "26m"; "<1m" below a minute.
pub fn countdown(ms: i64) -> String {
    let minutes = ms.div_euclid(MINUTE_MS).max(0);
    if minutes < 1 {
        return "<1m".to_string();
    }
    let days = minutes / 1440;
    let hours = (minutes % 1440) / 60;
    if days > 0 {
        return if hours > 0 { format!("{days}d {hours}h") } else { format!("{days}d") };
    }
    let mins = minutes % 60;
    if hours > 0 {
        return if mins > 0 { format!("{hours}h {mins}m") } else { format!("{hours}h") };
    }
    format!("{mins}m")
}

/// How long ago something happened: "just now", "5 min ago", "2 h ago", "3 d ago".
pub fn ago(ms: i64) -> String {
    let minutes = ms.div_euclid(MINUTE_MS);
    if minutes < 1 {
        return "just now".to_string();
    }
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} h ago");
    }
    format!("{} d ago", hours / 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;

    #[test]
    fn shows_the_time_left_in_the_two_largest_units() {
        assert_eq!(countdown(30_000), "<1m");
        assert_eq!(countdown(26 * MINUTE), "26m");
        assert_eq!(countdown(166 * MINUTE), "2h 46m");
        assert_eq!(countdown(120 * MINUTE), "2h");
        assert_eq!(countdown((3 * 24 + 8) * 60 * MINUTE), "3d 8h");
        assert_eq!(countdown(-5 * MINUTE), "<1m");
        assert_eq!(countdown(2 * 24 * 60 * MINUTE), "2d");
    }

    #[test]
    fn says_how_long_ago_something_happened() {
        assert_eq!(ago(20_000), "just now");
        assert_eq!(ago(5 * MINUTE), "5 min ago");
        assert_eq!(ago(59 * MINUTE), "59 min ago");
        assert_eq!(ago(130 * MINUTE), "2 h ago");
        assert_eq!(ago(3 * 24 * 60 * MINUTE), "3 d ago");
    }
}
