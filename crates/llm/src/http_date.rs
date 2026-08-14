//! HTTP-date parsing helpers for the `Retry-After` header.

/// Parse an RFC 7231 IMF-fixdate (e.g. "Sun, 06 Nov 1994 08:49:37 GMT") and
/// return the number of seconds from now until that date (0 if in the past).
pub(crate) fn parse_http_date_to_secs(s: &str) -> Option<u64> {
    // Format: "Day, DD Mon YYYY HH:MM:SS GMT"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 6 {
        return None;
    }
    // parts[0] = "Day," (ignore), parts[1] = day, parts[2] = month,
    // parts[3] = year, parts[4] = time, parts[5] = "GMT"
    let day: u64 = parts[1].trim_end_matches(',').parse().ok()?;
    let month = MONTHS.iter().position(|m| *m == parts[2])? as u64 + 1;
    let year: u64 = parts[3].parse().ok()?;
    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u64 = time_parts[0].parse().ok()?;
    let minute: u64 = time_parts[1].parse().ok()?;
    let second: u64 = time_parts[2].parse().ok()?;
    if parts[5] != "GMT" {
        return None;
    }

    let target = days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(target.saturating_sub(now))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Days since UNIX epoch (1970-01-01) for a civil date.
/// Uses Howard Hinnant's algorithm: https://howardhinnant.github.io/date_algorithms.html
fn days_from_civil(y: u64, m: u64, d: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let m = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
