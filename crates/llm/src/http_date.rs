//! HTTP-date parsing helpers for the `Retry-After` header.

use std::time::{SystemTime, UNIX_EPOCH};

/// Parse an RFC 7231 IMF-fixdate (e.g. "Sun, 06 Nov 1994 08:49:37 GMT") and
/// return the number of seconds from now until that date (0 if in the past).
pub(crate) fn parse_http_date_to_secs(s: &str) -> Option<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    http_date_secs_since(s, now)
}

/// Pure core of [`parse_http_date_to_secs`]: seconds from `now` until the
/// parsed date. A date at or before `now` yields `Some(0)` — never an
/// underflow panic (debug) or wrapped astronomical value (release).
fn http_date_secs_since(s: &str, now: u64) -> Option<u64> {
    // Format: "Day, DD Mon YYYY HH:MM:SS GMT"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 6 {
        return None;
    }
    // parts[0] = "Day," (ignore), parts[1] = day, parts[2] = month,
    // parts[3] = year, parts[4] = time, parts[5] = "GMT"
    // Day/year parse as i64: dates before 1970 need signed day arithmetic.
    let day: i64 = parts[1].trim_end_matches(',').parse().ok()?;
    let month = MONTHS.iter().position(|m| *m == parts[2])? as i64 + 1;
    let year: i64 = parts[3].parse().ok()?;
    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: i64 = time_parts[0].parse().ok()?;
    let minute: i64 = time_parts[1].parse().ok()?;
    let second: i64 = time_parts[2].parse().ok()?;
    if parts[5] != "GMT" {
        return None;
    }
    // Field-range validation: reject structural nonsense (day "00", hour 25,
    // leap-second-style ":60") before it reaches the arithmetic. The month is
    // already validated by table lookup above.
    if !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    // Checked arithmetic: an absurd year must overflow to `None`, not panic.
    let target = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600)?
        .checked_add(minute * 60)?
        .checked_add(second)?;
    let delta = (target - i64::try_from(now).unwrap_or(i64::MAX)).max(0);
    Some(u64::try_from(delta).unwrap_or(u64::MAX))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since the UNIX epoch (1970-01-01) for a civil date. Signed: dates
/// before 1970 yield negative values instead of underflowing.
/// Uses Howard Hinnant's algorithm: https://howardhinnant.github.io/date_algorithms.html
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- pre-1970 dates: signed arithmetic, never underflow ----

    /// Regression: the old all-`u64` `days_from_civil` underflowed on
    /// `era * 146097 + doe - 719468` (panic in debug, wrap in release).
    #[test]
    fn pre_1970_date_yields_zero_without_underflow() {
        assert_eq!(
            http_date_secs_since("Mon, 01 Jan 1900 00:00:00 GMT", 0),
            Some(0)
        );
        assert_eq!(
            http_date_secs_since("Mon, 01 Jan 1900 00:00:00 GMT", 784_111_777),
            Some(0)
        );
    }

    #[test]
    fn year_zero_yields_zero_without_underflow() {
        assert_eq!(
            http_date_secs_since("Sat, 01 Jan 0000 00:00:00 GMT", 1),
            Some(0)
        );
    }

    // ---- exact epoch math via the pure core (deterministic) ----

    #[test]
    fn epoch_itself_at_now_zero_is_zero() {
        assert_eq!(
            http_date_secs_since("Thu, 01 Jan 1970 00:00:00 GMT", 0),
            Some(0)
        );
    }

    #[test]
    fn next_day_after_epoch_is_86400() {
        assert_eq!(
            http_date_secs_since("Fri, 02 Jan 1970 00:00:00 GMT", 0),
            Some(86_400)
        );
    }

    /// The canonical RFC 7231 example: 1994-11-06T08:49:37Z == 784111777.
    #[test]
    fn rfc7231_example_is_exact_epoch() {
        assert_eq!(
            http_date_secs_since("Sun, 06 Nov 1994 08:49:37 GMT", 0),
            Some(784_111_777)
        );
        assert_eq!(
            http_date_secs_since("Sun, 06 Nov 1994 08:49:37 GMT", 784_111_777),
            Some(0)
        );
        assert_eq!(
            http_date_secs_since("Sun, 06 Nov 1994 08:49:37 GMT", 784_111_776),
            Some(1)
        );
    }

    /// Far-future dates still resolve (nonzero) with exact epoch values,
    /// including leap years (2000) and the non-leap century year 2100.
    #[test]
    fn civil_calendar_matches_reference_epochs() {
        for (date, epoch) in [
            ("Sat, 01 Jan 2000 00:00:00 GMT", 946_684_800),
            ("Wed, 01 Mar 2000 00:00:00 GMT", 951_868_800),
            ("Mon, 01 Mar 2100 00:00:00 GMT", 4_107_542_400),
            ("Fri, 01 Jan 2999 00:00:00 GMT", 32_472_144_000),
        ] {
            assert_eq!(http_date_secs_since(date, 0), Some(epoch), "{date}");
        }
    }

    // ---- field-range validation ----

    #[test]
    fn out_of_range_fields_are_rejected() {
        for bad in [
            "Sun, 00 Nov 1994 08:49:37 GMT", // day 00
            "Sun, 32 Nov 1994 08:49:37 GMT", // day 32
            "Sun, 06 Nov 1994 24:49:37 GMT", // hour 24
            "Sun, 06 Nov 1994 08:60:37 GMT", // minute 60
            "Sun, 06 Nov 1994 08:49:60 GMT", // second 60
            "Sun, 06 Foo 1994 08:49:37 GMT", // month not in table
        ] {
            assert_eq!(http_date_secs_since(bad, 0), None, "{bad}");
        }
    }

    /// A year large enough to overflow `days * 86400` must yield `None`
    /// (checked arithmetic), not a debug-build overflow panic.
    #[test]
    fn absurd_years_overflow_to_none() {
        assert_eq!(
            http_date_secs_since("Fri, 01 Jan 999999999999999 00:00:00 GMT", 0),
            None
        );
        assert_eq!(
            http_date_secs_since("Fri, 01 Jan -999999999999999 00:00:00 GMT", 0),
            None
        );
    }

    // ---- SystemTime wrapper sanity ----

    #[test]
    fn wrapper_reports_past_dates_as_zero() {
        assert_eq!(
            parse_http_date_to_secs("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(0)
        );
    }
}
