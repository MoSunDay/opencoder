//! Cron-scheduling primitives shared by config validation (`config/schedule.rs`),
//! the control-plane scheduler (`opencoder-control`) and future local reuse.
//! Pure functions over `chrono`/`cron`; no tokio, no I/O.

pub mod cron;
pub mod time_param;

pub use cron::{parse_timezone, CronExpr};
pub use time_param::render_params;

/// Unix-ms → `DateTime<Utc>`; out-of-range inputs clamp to now (callers pass
/// code-computed values, so this is a belt-and-braces guard).
pub fn to_utc(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(chrono::Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cron_and_params_compose_into_a_validated_schedule_unit() {
        // The two primitives compose the exact way the scheduler uses them:
        // compute the fire instant from the cron, then render params at it.
        let base = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 17, 0, 0, 0).unwrap();
        let expr = CronExpr::parse("0 3 * * *", Some("+08:00")).unwrap();
        let fire = expr.next_after(base).unwrap();
        assert_eq!(
            fire,
            chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 9, 17, 19, 0, 0).unwrap()
        );
        let tz = parse_timezone("+08:00").unwrap();
        let params = render_params(
            &json!({"title": "日报 {{now-1d:%Y-%m-%d}}"}),
            fire.with_timezone(&tz),
        )
        .unwrap();
        assert_eq!(params["title"], "日报 2026-09-17");
    }
}
