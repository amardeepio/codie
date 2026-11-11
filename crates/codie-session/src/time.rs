//! RFC 3339 timestamp formatting without a calendar dependency.

use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current time as an RFC 3339 string with millisecond precision.
///
/// The timestamp is always UTC and ends with `Z`, for example
/// `2026-11-11T09:30:00.123Z`. Session entry timestamps are required to be
/// RFC 3339 strings (SESS-001, MSG-010).
pub(crate) fn rfc3339_now() -> String {
    rfc3339_from_system_time(SystemTime::now())
}

/// Formats a [`SystemTime`] as an RFC 3339 UTC string with millisecond
/// precision.
pub(crate) fn rfc3339_from_system_time(time: SystemTime) -> String {
    let (secs, nanos) = unix_parts(time);
    format_rfc3339(secs, nanos)
}

/// Returns the current time in Unix nanoseconds, saturating at zero.
pub(crate) fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            let nanos = duration.as_nanos();
            u64::try_from(nanos).unwrap_or(u64::MAX)
        })
}

/// Splits a system time into whole seconds and a subsecond nanosecond count.
fn unix_parts(time: SystemTime) -> (i64, u32) {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let secs = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
            (secs, duration.subsec_nanos())
        }
        Err(error) => {
            let duration = error.duration();
            let secs = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
            let nanos = duration.subsec_nanos();
            if nanos == 0 {
                (-secs, 0)
            } else {
                // Move back into the previous whole second.
                (secs.saturating_neg() - 1, 1_000_000_000 - nanos)
            }
        }
    }
}

/// Formats Unix seconds plus nanoseconds as RFC 3339 UTC.
fn format_rfc3339(secs: i64, nanos: u32) -> String {
    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let millis = nanos / 1_000_000;
    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Converts a day count since the Unix epoch to a civil date.
///
/// This is Howard Hinnant's `civil_from_days` algorithm, which is valid for the
/// full range of `i64` days that fits a `SystemTime`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = u64::try_from(shifted - era * 146_097).unwrap_or(0);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = i64::try_from(year_of_era).unwrap_or(0) + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * month_prime + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    })
    .unwrap_or(1);
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64, nanos: u32) -> String {
        format_rfc3339(i64::try_from(secs).unwrap(), nanos)
    }

    #[test]
    fn formats_a_known_epoch_timestamp() {
        // 2026-11-11T00:00:00Z
        assert_eq!(at(1_794_355_200, 0), "2026-11-11T00:00:00.000Z");
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn includes_millisecond_precision() {
        // 2025-11-13T02:13:20Z
        assert_eq!(at(1_763_000_000, 123_456_789), "2025-11-13T02:13:20.123Z");
    }

    #[test]
    fn handles_leap_days() {
        // 2024-02-29T12:34:56Z
        assert_eq!(at(1_709_210_096, 0), "2024-02-29T12:34:56.000Z");
    }

    #[test]
    fn handles_times_before_the_epoch() {
        let time = UNIX_EPOCH - Duration::from_millis(1_500);
        assert_eq!(rfc3339_from_system_time(time), "1969-12-31T23:59:58.500Z");
    }
}
