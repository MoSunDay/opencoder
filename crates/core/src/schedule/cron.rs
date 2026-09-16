//! Cron expression parsing + next-fire computation (pure, no tokio).
//!
//! Expressions use the familiar 5-field form `分 时 日 月 周`; an optional
//! leading seconds field (6 fields) is accepted for sub-minute schedules —
//! it is what keeps scheduler tests fast. Timezones are fixed offsets only
//! (no DST; `chrono-tz` is deliberately out of scope).

use chrono::{DateTime, FixedOffset, Utc};

/// A parsed cron schedule bound to a fixed timezone.
#[derive(Debug, Clone, PartialEq)]
pub struct CronExpr {
    schedule: cron::Schedule,
    tz: FixedOffset,
}

impl CronExpr {
    /// Parse `expr` (5 fields: 分 时 日 月 周; 6/7 fields keep the cron crate's
    /// seconds-first form) with an optional fixed-offset timezone spec.
    pub fn parse(expr: &str, tz: Option<&str>) -> Result<Self, String> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Err("cron expression is empty".into());
        }
        let fields = trimmed.split_whitespace().count();
        let normalized = match fields {
            5 => format!("0 {trimmed}"),
            6 | 7 => trimmed.to_string(),
            count => {
                let _ = count;
                return Err(format!(
                    "cron expression must have 5 fields (分 时 日 月 周, optionally with a \
                     leading seconds field), got {fields}: {expr:?}"
                ));
            }
        };
        let schedule = normalized
            .parse::<cron::Schedule>()
            .map_err(|error| format!("invalid cron expression {expr:?}: {error}"))?;
        let tz = parse_timezone(tz.unwrap_or("UTC"))?;
        Ok(Self { schedule, tz })
    }

    /// First fire strictly after `after` (the baseline is never re-fired),
    /// resolved in the schedule's timezone and returned as UTC.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.upcoming(after, 1).into_iter().next()
    }

    /// Up to `limit` fires strictly after `after`, ascending. `limit == 0`
    /// yields an empty vec.
    pub fn upcoming(&self, after: DateTime<Utc>, limit: usize) -> Vec<DateTime<Utc>> {
        if limit == 0 {
            return Vec::new();
        }
        let local = after.with_timezone(&self.tz);
        self.schedule
            .after(&local)
            .take(limit)
            .map(|fire| fire.with_timezone(&Utc))
            .collect()
    }
}

/// Parse a fixed-offset timezone: `UTC`/`Z` or `±HH[:MM]`/`±HHMM`.
pub fn parse_timezone(spec: &str) -> Result<FixedOffset, String> {
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("utc") || spec.eq_ignore_ascii_case("z") {
        return Ok(FixedOffset::east_opt(0).expect("zero offset"));
    }
    let (sign, rest) = match spec.as_bytes().first() {
        Some(b'+') => (1i32, &spec[1..]),
        Some(b'-') => (-1i32, &spec[1..]),
        _ => {
            return Err(format!(
                "invalid timezone {spec:?}: expected UTC, Z or ±HH:MM"
            ))
        }
    };
    let (hours, minutes) = match rest.split_once(':') {
        Some((h, m)) => (h, m),
        None if rest.len() == 4 => (&rest[..2], &rest[2..]),
        None => (rest, "0"),
    };
    let hours: i32 = hours_of(sign, hours)?;
    let minutes: i32 = minutes_of(minutes)?;
    let seconds = sign * (hours * 3600 + minutes * 60);
    FixedOffset::east_opt(seconds).ok_or_else(|| format!("timezone {spec:?} out of range"))
}

fn hours_of(sign: i32, raw: &str) -> Result<i32, String> {
    let hours: i32 = raw
        .parse()
        .map_err(|_| format!("invalid timezone hours {raw:?}"))?;
    if !(0..=23).contains(&hours) {
        return Err(format!("timezone hours {hours} out of range 0-23"));
    }
    Ok(sign.abs() * hours)
}

fn minutes_of(raw: &str) -> Result<i32, String> {
    let minutes: i32 = raw
        .parse()
        .map_err(|_| format!("invalid timezone minutes {raw:?}"))?;
    if !(0..=59).contains(&minutes) {
        return Err(format!("timezone minutes {minutes} out of range 0-59"));
    }
    Ok(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, s).unwrap()
    }

    #[test]
    fn five_field_expression_gains_zero_seconds() {
        let expr = CronExpr::parse("0 3 * * *", None).unwrap();
        let next = expr.next_after(utc(2026, 9, 17, 1, 0, 0)).unwrap();
        assert_eq!(next, utc(2026, 9, 17, 3, 0, 0));
    }

    #[test]
    fn five_field_uses_the_schedule_timezone() {
        // 03:00 +08:00 == 19:00 UTC the previous day.
        let expr = CronExpr::parse("0 3 * * *", Some("+08:00")).unwrap();
        let next = expr.next_after(utc(2026, 9, 17, 0, 0, 0)).unwrap();
        assert_eq!(next, utc(2026, 9, 17, 19, 0, 0));
    }

    #[test]
    fn six_field_seconds_expression_enables_fast_schedules() {
        let expr = CronExpr::parse("*/10 * * * * *", None).unwrap();
        let next = expr.next_after(utc(2026, 1, 1, 0, 0, 5)).unwrap();
        assert_eq!(next, utc(2026, 1, 1, 0, 0, 10));
    }

    #[test]
    fn next_after_excludes_the_baseline_tick() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let baseline = utc(2026, 5, 1, 12, 30, 0);
        // A baseline that IS a tick must not be returned again.
        assert_eq!(
            expr.next_after(baseline).unwrap(),
            utc(2026, 5, 1, 12, 31, 0)
        );
    }

    #[test]
    fn upcoming_returns_ascending_limited_fires() {
        let expr = CronExpr::parse("*/15 * * * *", None).unwrap();
        let fires = expr.upcoming(utc(2026, 1, 1, 0, 0, 0), 3);
        assert_eq!(
            fires,
            vec![
                utc(2026, 1, 1, 0, 15, 0),
                utc(2026, 1, 1, 0, 30, 0),
                utc(2026, 1, 1, 0, 30, 0).with_minute(45).unwrap(),
            ]
        );
        assert!(expr.upcoming(utc(2026, 1, 1, 0, 0, 0), 0).is_empty());
    }

    #[test]
    fn rejects_wrong_field_counts_and_garbage() {
        assert!(CronExpr::parse("* * *", None).is_err());
        assert!(CronExpr::parse("* * * * * * * *", None).is_err());
        assert!(CronExpr::parse("nope", None).is_err());
        assert!(CronExpr::parse("", None).is_err());
        assert!(CronExpr::parse("99 3 * * *", None).is_err());
    }

    #[test]
    fn timezone_forms_are_accepted_and_rejected() {
        assert_eq!(parse_timezone("UTC").unwrap().local_minus_utc(), 0);
        assert_eq!(parse_timezone("z").unwrap().local_minus_utc(), 0);
        assert_eq!(
            parse_timezone("+08:00").unwrap().local_minus_utc(),
            8 * 3600
        );
        assert_eq!(
            parse_timezone("-0530").unwrap().local_minus_utc(),
            -(5 * 3600 + 1800)
        );
        assert_eq!(parse_timezone("+08").unwrap().local_minus_utc(), 8 * 3600);
        assert!(parse_timezone("+25:00").is_err());
        assert!(parse_timezone("Asia/Shanghai").is_err());
        assert!(parse_timezone("+08:99").is_err());
    }
}
