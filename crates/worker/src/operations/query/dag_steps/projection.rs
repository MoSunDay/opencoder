use opencoder_store::store::dag_snapshot::DagStepEvent;
use serde_json::{json, Value};

/// A new attempt supersedes the previous receipt. A receipt can land before
/// its queued event; retain its timestamp so the browser rejects that older
/// step_started event when it later arrives above the snapshot watermark.
pub(super) fn project(name: &str, meta: &Value, event: Option<&DagStepEvent>, run: &str) -> Value {
    let receipt_at = meta["finished_at_ms"].as_i64().unwrap_or(0);
    let receipt_current = !meta.is_null()
        && event.is_none_or(|e| {
            if e.started {
                return receipt_at >= e.at_ms;
            }
            // Completed events are emitted AFTER writing the receipt. The
            // receipt distinguishes cancellation from failure, but a receipt
            // from an older attempt or a later persistence error cannot win.
            receipt_at > e.at_ms
                || (receipt_at >= e.started_at_ms
                    && e.ok == (super::outcome_status(meta) == "done")
                    && e.error.as_deref() == meta["error"].as_str())
        });
    let (status, error, at_ms) = if receipt_current {
        (
            super::outcome_status(meta),
            meta["error"].clone(),
            receipt_at,
        )
    } else if let Some(event) = event {
        let status = if event.started {
            match run {
                "pending" | "running" | "cancelling" => "running",
                "interrupted" | "cancelled" => run,
                _ => "error",
            }
        } else if event.ok {
            "done"
        } else {
            "error"
        };
        (status, json!(event.error), event.at_ms)
    } else {
        ("pending", Value::Null, 0)
    };
    json!({"name":name,"status":status,"error":error,"at_ms":at_ms,
        "seq":event.map_or(0, |e| e.seq)})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(at_ms: i64) -> DagStepEvent {
        DagStepEvent {
            name: "build".into(),
            seq: 5,
            started: true,
            at_ms,
            started_at_ms: at_ms,
            ok: true,
            error: None,
        }
    }

    #[test]
    fn receipt_and_retry_are_ordered_by_attempt_time() {
        let receipt = json!({"outcome":"error","finished_at_ms":20,"error":"old failure"});
        assert_eq!(
            project("build", &receipt, Some(&started(10)), "running")["status"],
            "error"
        );
        let retry = project("build", &receipt, Some(&started(30)), "running");
        assert_eq!(retry["status"], "running");
        assert!(retry["error"].is_null());
    }

    #[test]
    fn running_and_stopped_steps_do_not_appear_pending() {
        for status in ["running", "interrupted", "cancelled"] {
            assert_eq!(
                project("build", &Value::Null, Some(&started(10)), status)["status"],
                status
            );
        }
        assert_eq!(
            project("build", &Value::Null, None, "running")["status"],
            "pending"
        );
    }

    #[test]
    fn cancelled_receipt_survives_its_later_completion_event() {
        let meta = json!({"outcome":"cancelled","finished_at_ms":20,"error":"cancelled"});
        let event = DagStepEvent {
            started: false,
            at_ms: 30,
            ok: false,
            error: Some("cancelled".into()),
            ..started(10)
        };
        assert_eq!(
            project("build", &meta, Some(&event), "cancelled")["status"],
            "cancelled"
        );
        let later_attempt = DagStepEvent {
            started_at_ms: 25,
            ..event
        };
        assert_eq!(
            project("build", &meta, Some(&later_attempt), "error")["status"],
            "error"
        );
    }

    #[test]
    fn artifact_failure_after_a_receipt_is_not_hidden() {
        let meta = json!({"outcome":"done","finished_at_ms":20});
        let event = DagStepEvent {
            started: false,
            at_ms: 20,
            ok: false,
            error: Some("step artifact persistence failed".into()),
            ..started(10)
        };
        let result = project("build", &meta, Some(&event), "error");
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"], "step artifact persistence failed");
    }
}
