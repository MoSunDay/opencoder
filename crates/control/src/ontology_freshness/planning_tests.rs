//! Unit tests for the pure planning helpers (state-free by construction).
use super::*;
use opencoder_core::fleet::{valid_id, ExecutionStatus};
use opencoder_ontology::{FreshnessItemStatus, FreshnessRunItem};
use serde_json::json;
use uuid::Uuid;

fn entity() -> serde_json::Value {
    json!({
        "id": "0b0b0b0b-0b0b-4b0b-8b0b-0b0b0b0b0b0b",
        "name": "billing dataset",
        "description": "nightly ledger export",
        "data_level": "fresh"
    })
}

fn item(status: FreshnessItemStatus) -> FreshnessRunItem {
    FreshnessRunItem {
        entity_id: Uuid::new_v4(),
        status,
        ..FreshnessRunItem::default()
    }
}

#[test]
fn execution_ids_are_deterministic_distinct_and_fleet_legal() {
    let run = Uuid::new_v4();
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    let id = execution_id("agent", run, left);
    assert_eq!(id, execution_id("agent", run, left));
    assert_ne!(id, execution_id("agent", run, right));
    assert_ne!(id, execution_id("dag", run, left));
    for candidate in [id, execution_id("dag", run, left)] {
        let prefix = candidate.split('-').next().unwrap();
        assert!(valid_id(&candidate), "{candidate}");
        assert_eq!(candidate.len(), prefix.len() + 1 + 24);
    }
}

#[test]
fn render_prompt_substitutes_known_and_blanks_unknown_fields() {
    let template = "{{entity.name}}|{{entity.description}}|{{entity.data_level}}|{{entity.id}}|{{entity.secret}}";
    assert_eq!(
        render_prompt(template, &entity()),
        "billing dataset|nightly ledger export|fresh|0b0b0b0b-0b0b-4b0b-8b0b-0b0b0b0b0b0b|"
    );
    // Missing fields and unterminated placeholders degrade instead of leaking.
    let sparse = json!({"name": "alpha"});
    assert_eq!(render_prompt("{{entity.name}}: {{entity.description}}", &sparse), "alpha: ");
    assert_eq!(render_prompt("{{entity.name", &sparse), "{{entity.name");
    assert_eq!(render_prompt("no placeholders", &sparse), "no placeholders");
}

fn policy(kind: &str, template: &str) -> serde_json::Value {
    json!({
        "entity_type_id": "1e1e1e1e-1e1e-4e1e-8e1e-1e1e1e1e1e1e",
        "concurrency": 5,
        "executor": {"kind": kind, "target": "agent://refresher", "prompt_template": template}
    })
}

#[test]
fn execution_input_freezes_agent_and_dag_shapes() {
    let rendered = execution_input(&policy("agent", "refresh {{entity.name}}"), &entity());
    assert_eq!(rendered, json!({"prompt": "refresh billing dataset"}));
    // Blank templates fall back to the entity name and description.
    let fallback = execution_input(&policy("agent", "  "), &entity());
    assert_eq!(
        fallback["prompt"].as_str().unwrap(),
        "Refresh entity billing dataset: nightly ledger export"
    );
    let dag = execution_input(&policy("dag", "refresh {{entity.name}}"), &entity());
    assert_eq!(dag["entity"], entity());
    assert_eq!(dag["prompt"], json!("refresh billing dataset"));
    let dag_blank = execution_input(&policy("dag", ""), &entity());
    assert_eq!(dag_blank["prompt"], serde_json::Value::Null);
    assert_eq!(dag_blank["entity"], entity());
}

#[test]
fn concurrency_and_target_are_sanitized() {
    for (declared, expected) in [(0, 1), (1, 1), (5, 5), (30, 30), (99, 30)] {
        assert_eq!(concurrency(&json!({ "concurrency": declared })), expected);
    }
    assert_eq!(concurrency(&json!({})), 1);
    assert_eq!(
        executor_target(&policy("agent", "")),
        Some("agent://refresher".into())
    );
    assert_eq!(executor_target(&json!({"executor":{"target":"  "}})), None);
    assert_eq!(executor_kind(&policy("dag", "")), ExecutionKind::Dag);
    assert_eq!(executor_kind(&policy("agent", "")), ExecutionKind::Agent);
    assert_eq!(executor_kind(&json!({})), ExecutionKind::Agent);
}

#[test]
fn settle_matches_the_data_plane_rule() {
    assert_eq!(settle(&[]), FreshnessRunStatus::Failed);
    assert_eq!(
        settle(&vec![item(FreshnessItemStatus::Succeeded); 2]),
        FreshnessRunStatus::Succeeded
    );
    assert_eq!(
        settle(&vec![item(FreshnessItemStatus::Failed); 2]),
        FreshnessRunStatus::Failed
    );
    let mixed = vec![
        item(FreshnessItemStatus::Succeeded),
        item(FreshnessItemStatus::Failed),
    ];
    assert_eq!(settle(&mixed), FreshnessRunStatus::Partial);
}

#[test]
fn transitions_allow_only_queued_start_and_running_finish() {
    use FreshnessRunStatus::*;
    assert!(transition_allowed(Queued, Running));
    for finish in [Succeeded, Partial, Failed] {
        assert!(transition_allowed(Running, finish));
        assert!(!transition_allowed(Queued, finish));
        assert!(!transition_allowed(finish, Running));
    }
    assert!(!transition_allowed(Running, Running));
}

#[test]
fn merge_progress_moves_items_forward_only_by_entity_id() {
    let entity = Uuid::new_v4();
    let mut stored = vec![
        FreshnessRunItem {
            entity_id: entity,
            status: FreshnessItemStatus::Pending,
            ..FreshnessRunItem::default()
        },
        item(FreshnessItemStatus::Dispatched),
    ];
    let local = vec![FreshnessRunItem {
        entity_id: entity,
        status: FreshnessItemStatus::Failed,
        error: Some("submit 503".into()),
        ..FreshnessRunItem::default()
    }];
    merge_progress(&mut stored, &local);
    assert_eq!(stored[0].status, FreshnessItemStatus::Failed);
    assert_eq!(stored[0].error.as_deref(), Some("submit 503"));
    assert_eq!(stored[1].status, FreshnessItemStatus::Dispatched);
    // Stale local state never overwrites stored progress.
    let mut stored = vec![FreshnessRunItem {
        entity_id: entity,
        status: FreshnessItemStatus::Dispatched,
        execution_id: Some("agent-1".into()),
        ..FreshnessRunItem::default()
    }];
    let stale = vec![FreshnessRunItem {
        entity_id: entity,
        status: FreshnessItemStatus::Pending,
        ..FreshnessRunItem::default()
    }];
    merge_progress(&mut stored, &stale);
    assert_eq!(stored[0].status, FreshnessItemStatus::Dispatched);
    assert_eq!(stored[0].execution_id.as_deref(), Some("agent-1"));
}

#[test]
fn poll_and_read_outcomes_map_statuses_and_bounds() {
    assert!(matches!(poll_from_index(ExecutionStatus::Running), PollOutcome::Pending));
    assert!(matches!(poll_from_index(ExecutionStatus::Done), PollOutcome::Done));
    for failed in [ExecutionStatus::Error, ExecutionStatus::Cancelled] {
        assert!(matches!(
            poll_from_index(failed),
            PollOutcome::Failed((FreshnessItemStatus::Failed, None, Some(_)))
        ));
    }
    let (status, candidate, error) = outcome_from_read(200, &json!({"summary": " ok "}));
    assert_eq!(status, FreshnessItemStatus::Succeeded);
    assert_eq!(candidate.as_deref(), Some(" ok "));
    assert_eq!(error, None);
    let (status, candidate, error) = outcome_from_read(200, &json!({"summary": null}));
    assert_eq!(error, None);
    assert_eq!(status, FreshnessItemStatus::Succeeded);
    assert_eq!(candidate, None);
    let (status, _, error) = outcome_from_read(404, &json!({"error": "execution not found"}));
    assert_eq!(status, FreshnessItemStatus::Failed);
    assert!(error.unwrap().starts_with("result 404: "));
    // Bounds protect the stored record size.
    let long = "ab".repeat(MAX_ERROR_CHARS + 10);
    assert_eq!(bound(&long, MAX_ERROR_CHARS).chars().count(), MAX_ERROR_CHARS);
}
