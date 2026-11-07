//! A QuickLook block is a passing thought: it goes by itself once its time
//! is up, an hour after it was made unless the user picks another span or
//! keeps it. In its last minutes it fades; ⌘Z brings it back with a little
//! more time.

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;

/// How long a new QuickLook block stays.
pub const NOTE_LIFE_MS: i64 = HOUR_MS;
/// How long before it goes a QuickLook starts fading, and how faint it gets.
pub const NOTE_FADE_MS: i64 = 10 * MINUTE_MS;
pub const NOTE_FADE_TO: f32 = 0.4;
/// The time a QuickLook brought back by an undo has left.
pub const NOTE_RESCUE_MS: i64 = 10 * MINUTE_MS;

/// The spans the time-left menu offers; 0 keeps the block.
pub const LIFE_CHOICES: [(&str, i64); 7] = [
    ("10 分钟", 10 * MINUTE_MS),
    ("30 分钟", 30 * MINUTE_MS),
    ("1 小时", HOUR_MS),
    ("3 小时", 3 * HOUR_MS),
    ("1 天", 24 * HOUR_MS),
    ("1 周", 7 * 24 * HOUR_MS),
    ("永久保留", 0),
];

/// How much a QuickLook shows: fully until it starts fading, then less.
pub fn note_opacity(expires: i64, now: i64) -> f32 {
    if expires <= 0 {
        return 1.0;
    }
    let left = expires - now;
    if left >= NOTE_FADE_MS {
        return 1.0;
    }
    let f = left.max(0) as f32 / NOTE_FADE_MS as f32;
    NOTE_FADE_TO + (1.0 - NOTE_FADE_TO) * f
}

/// Reads a time left: minutes up to an hour, then hours, then days.
pub fn life_left(left_ms: i64) -> String {
    let mins = left_ms as f64 / MINUTE_MS as f64;
    if left_ms <= MINUTE_MS {
        "< 1 分钟".into()
    } else if left_ms <= HOUR_MS {
        format!("{} 分钟", mins.ceil() as i64)
    } else if left_ms < 48 * HOUR_MS {
        format!("{} 小时", (mins / 60.0).round() as i64)
    } else {
        format!("{} 天", (mins / 60.0 / 24.0).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left_reads_in_the_largest_unit() {
        assert_eq!(life_left(30_000), "< 1 分钟");
        assert_eq!(life_left(59 * MINUTE_MS + 1), "60 分钟");
        assert_eq!(life_left(5 * HOUR_MS), "5 小时");
        assert_eq!(life_left(72 * HOUR_MS), "3 天");
    }

    #[test]
    fn notes_fade_in_their_last_minutes() {
        assert_eq!(note_opacity(0, 5), 1.0);
        assert_eq!(note_opacity(HOUR_MS, 0), 1.0);
        assert!((note_opacity(NOTE_FADE_MS / 2, 0) - 0.7).abs() < 1e-5);
        assert_eq!(note_opacity(10, 20), NOTE_FADE_TO);
    }
}
