//! Pure cron timing decisions bounded by the current stored definition.
use opencoder_core::schedule::{to_utc, CronExpr};
use opencoder_store::{ScheduleDefRecord, ScheduleRunRecord, SCHEDULE_RUN_ERROR};

/// How far back to look for due ticks after server downtime. Within this
/// window only the newest tick fires; older due ticks collapse as `missed`.
const CATCHUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// An errored fire retries on later scans inside this window, then waits
/// for the next cron tick instead of spinning.
const RETRY_WINDOW_MS: i64 = 60 * 60 * 1000;
/// Hard cap on candidate ticks per scan: the 24h walk-back of the densest
/// sane cron (every minute) still fits, denser 6-field crons collapse.
const MAX_CANDIDATE_TICKS: usize = 2_000;
/// One scan submits at most one tick and may audit an older skipped tick.
#[derive(Debug, PartialEq)]
pub(super) struct DueTick {
    pub scheduled_for_ms: i64,
    pub missed_for_ms: Option<i64>,
}

/// Creation, edits and re-enabling take effect from the stored revision.
/// Only unchanged definitions catch up across downtime or retry old errors.
pub(super) fn plan_due(
    expr: &CronExpr,
    def: &ScheduleDefRecord,
    last: Option<&ScheduleRunRecord>,
    now: i64,
) -> Option<DueTick> {
    let effective_since = def.created_at.max(def.updated_at);
    if let Some(last) = last {
        if last.status == SCHEDULE_RUN_ERROR
            && last.scheduled_for_ms >= effective_since
            && last.scheduled_for_ms <= now
            && now.saturating_sub(last.scheduled_for_ms) < RETRY_WINDOW_MS
        {
            return Some(DueTick {
                scheduled_for_ms: last.scheduled_for_ms,
                missed_for_ms: None,
            });
        }
    }
    let after = last
        .map(|run| run.scheduled_for_ms)
        .unwrap_or(effective_since)
        .max(effective_since)
        .max(now - CATCHUP_WINDOW_MS);
    let newest = due_ticks(expr, after, now).pop()?;
    let missed_for_ms = expr
        .next_after(to_utc(after))
        .map(|tick| tick.timestamp_millis())
        .filter(|first| *first < newest);
    Some(DueTick {
        scheduled_for_ms: newest,
        missed_for_ms,
    })
}

/// Newest due tick for the walk-back window, or `None`. Iterates the cron
/// forward from `after_ms`; when the window is denser than
/// [`MAX_CANDIDATE_TICKS`] the scan narrows toward `now` so the newest tick
/// is still found (dense-cron catch-up history is collapsed anyway).
fn due_ticks(expr: &CronExpr, after: i64, now: i64) -> Vec<i64> {
    let mut span = CATCHUP_WINDOW_MS;
    loop {
        let start = after.max(now - span);
        let upcoming = expr.upcoming(to_utc(start), MAX_CANDIDATE_TICKS + 1);
        let due: Vec<i64> = upcoming
            .iter()
            .map(|tick| tick.timestamp_millis())
            .take_while(|ms| *ms <= now)
            .collect();
        if due.len() <= MAX_CANDIDATE_TICKS || start >= now {
            return due;
        }
        // Truncated: a dense cron fills the scan. Zoom into the recent past.
        span /= 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed epoch: 2027-01-15T08:00:00Z, safely in the future.
    const NOW_MS: i64 = 1_800_000_000_000;

    fn definition(created_at: i64, updated_at: i64) -> ScheduleDefRecord {
        ScheduleDefRecord {
            job: serde_json::from_value(serde_json::json!({
                "id": "timing", "cron": "* * * * *", "kind": "agent", "target": "act"
            }))
            .unwrap(),
            created_at,
            updated_at,
        }
    }

    fn run(scheduled_for_ms: i64, status: &str) -> ScheduleRunRecord {
        ScheduleRunRecord {
            schedule_id: "timing".into(),
            kind: "agent".into(),
            target: "act".into(),
            scheduled_for_ms,
            fired_at_ms: scheduled_for_ms,
            execution_id: None,
            status: status.into(),
            error: None,
            missed: false,
        }
    }

    #[test]
    fn new_definition_waits_for_its_first_future_tick() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let created = NOW_MS + 1_000;
        let def = definition(created, created);
        assert_eq!(plan_due(&expr, &def, None, created + 15_000), None);
        assert_eq!(
            plan_due(&expr, &def, None, NOW_MS + 60_000),
            Some(DueTick {
                scheduled_for_ms: NOW_MS + 60_000,
                missed_for_ms: None,
            })
        );
        // Creating exactly at a tick still starts strictly after creation.
        assert_eq!(
            plan_due(&expr, &definition(NOW_MS, NOW_MS), None, NOW_MS),
            None
        );
    }

    #[test]
    fn edited_or_reenabled_definition_excludes_old_ticks_and_errors() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let def = definition(NOW_MS - CATCHUP_WINDOW_MS, NOW_MS + 1_000);
        for status in [opencoder_store::SCHEDULE_RUN_FIRED, SCHEDULE_RUN_ERROR] {
            let old = run(NOW_MS - 60_000, status);
            assert_eq!(plan_due(&expr, &def, Some(&old), NOW_MS + 15_000), None);
            assert_eq!(
                plan_due(&expr, &def, Some(&old), NOW_MS + 60_000),
                Some(DueTick {
                    scheduled_for_ms: NOW_MS + 60_000,
                    missed_for_ms: None,
                })
            );
        }
        assert_eq!(plan_due(&expr, &def, None, NOW_MS + 15_000), None);
    }

    #[test]
    fn downtime_catches_up_only_ticks_after_the_definition() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let def = definition(NOW_MS - 180_000, NOW_MS - 180_000);
        // Restart before the first ever dispatch still catches up.
        assert_eq!(
            plan_due(&expr, &def, None, NOW_MS),
            Some(DueTick {
                scheduled_for_ms: NOW_MS,
                missed_for_ms: Some(NOW_MS - 120_000),
            })
        );
        let last = run(NOW_MS - 120_000, opencoder_store::SCHEDULE_RUN_FIRED);
        assert_eq!(
            plan_due(&expr, &def, Some(&last), NOW_MS),
            Some(DueTick {
                scheduled_for_ms: NOW_MS,
                missed_for_ms: Some(NOW_MS - 60_000),
            })
        );
        let latest = run(NOW_MS, opencoder_store::SCHEDULE_RUN_FIRED);
        assert_eq!(plan_due(&expr, &def, Some(&latest), NOW_MS + 15_000), None);
    }

    #[test]
    fn unchanged_definition_retries_in_place_until_the_retry_deadline() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let def = definition(NOW_MS - CATCHUP_WINDOW_MS, NOW_MS - CATCHUP_WINDOW_MS);
        let last = run(NOW_MS - RETRY_WINDOW_MS + 1, SCHEDULE_RUN_ERROR);
        assert_eq!(
            plan_due(&expr, &def, Some(&last), NOW_MS),
            Some(DueTick {
                scheduled_for_ms: last.scheduled_for_ms,
                missed_for_ms: None,
            })
        );
        let expired = run(NOW_MS - RETRY_WINDOW_MS, SCHEDULE_RUN_ERROR);
        assert_eq!(
            plan_due(&expr, &def, Some(&expired), NOW_MS),
            Some(DueTick {
                scheduled_for_ms: NOW_MS,
                missed_for_ms: Some(expired.scheduled_for_ms + 60_000),
            })
        );
    }

    #[test]
    fn future_definitions_and_ledger_rows_never_fire_early() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let def = definition(NOW_MS + 1_000, NOW_MS + 1_000);
        assert_eq!(plan_due(&expr, &def, None, NOW_MS), None);
        let def = definition(NOW_MS - CATCHUP_WINDOW_MS, NOW_MS - CATCHUP_WINDOW_MS);
        let future = run(NOW_MS + 60_000, SCHEDULE_RUN_ERROR);
        assert_eq!(plan_due(&expr, &def, Some(&future), NOW_MS), None);
    }

    #[test]
    fn per_second_cron_zooms_to_the_newest_ticks() {
        let expr = CronExpr::parse("* * * * * *", None).unwrap();
        let now = NOW_MS;
        let after = now - CATCHUP_WINDOW_MS;
        let due = due_ticks(&expr, after, now);
        assert!(
            !due.is_empty(),
            "a due tick must be found in a dense window"
        );
        assert!(due.len() <= MAX_CANDIDATE_TICKS, "candidate cap must hold");
        // Ascending, within the (clipped) walk-back window, none in the future.
        assert!(due.windows(2).all(|w| w[0] < w[1]));
        assert!(due.iter().all(|ms| *ms <= now && *ms > after));
        // The zoom must land on recent ticks: the newest second boundary at or
        // before `now` — not the first 2000 seconds of the 24h walk-back.
        let newest = now - now.rem_euclid(1000);
        assert_eq!(*due.last().unwrap(), newest);
        assert!(*due.last().unwrap() > now - 1_400_000);
    }

    #[test]
    fn daily_cron_walks_back_within_the_catchup_window() {
        let expr = CronExpr::parse("0 5 * * *", None).unwrap();
        let now = NOW_MS; // 2027-01-15T08:00:00Z
                          // Walk-back clipped to 24h: only today's 05:00Z fires, never older ones.
        let due = due_ticks(&expr, now - 3 * CATCHUP_WINDOW_MS, now);
        assert_eq!(due, vec![1_799_989_200_000]); // 2027-01-15T05:00:00Z
                                                  // One day earlier the window starts exactly at that fire, which is
                                                  // exclusive: nothing due.
        let earlier = 1_799_989_200_000;
        assert!(due_ticks(&expr, earlier, earlier).is_empty());
    }

    #[test]
    fn at_or_after_now_yields_no_ticks() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let now = NOW_MS;
        assert!(due_ticks(&expr, now, now).is_empty());
        assert!(due_ticks(&expr, now + 3_600_000, now).is_empty());
    }
}
