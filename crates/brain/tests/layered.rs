//! Existing schema 4 data remains readable, but is never admitted as a new run.
use opencoder_brain::layered;
use opencoder_core::brain::layered::*;
use serde_json::json;
#[test]
fn historical_dag_can_be_drawn_without_enabling_legacy_execution() {
    let plan: LayeredPlan=serde_json::from_value(json!({"schema_version":4,"title":"old","objective":"old",
      "nodes":[{"node_id":"a","title":"first","capability_id":"agent"},{"node_id":"b","title":"second","capability_id":"agent"}],
      "edges":[{"from":"a","to":"b"}]})).unwrap();
    assert_eq!(layered::layers(&plan).unwrap(), vec![vec!["a"], vec!["b"]]);
    assert!(layered::validate_plan(&plan)
        .unwrap_err()
        .to_string()
        .contains("schema 5"));
}
#[test]
fn historical_run_and_events_default_new_metadata_only_for_reading() {
    let run:LayeredRun=serde_json::from_value(json!({"run_id":"brain-old","phase":"completed","layer":2,"generation":4,"last_event_seq":9,"error":null,"created_at":1,"updated_at":9})).unwrap();
    assert_eq!(run.activation, 0);
    assert_eq!(run.phase, LayeredPhase::Completed);
    let event:LayeredEvent=serde_json::from_value(json!({"seq":1,"run_id":"brain-old","layer":1,"event_type":"layer_started","evidence_execution_ids":[],"at_ms":1})).unwrap();
    assert!(event.assignments.is_empty());
    assert!(event.assessments.is_empty());
}
