//! Unit tests for the `schedules.json` domain config: parsing, validation
//! contracts, lenient entry handling and domain-file routing.

use serde_json::{json, Value};

use super::*;

fn job_json(kind: &str, target: &str, extra: Value) -> Value {
    let mut obj = json!({"id": "nightly", "kind": kind, "target": target});
    if let (Some(a), Some(b)) = (obj.as_object_mut(), extra.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    obj
}

fn validate_all(cfg: &SchedulesConfig) -> Result<(), String> {
    cfg.schedules.iter().try_for_each(|job| job.validate())
}

#[test]
fn empty_object_is_the_default_config() {
    let mut cfg = SchedulesConfig::default();
    let empty = serde_json::Map::new();
    merge(&mut cfg, &empty);
    assert!(cfg.schedules.is_empty());
    assert!(cfg.is_empty());
    assert_eq!(cfg.scan_interval_secs, None);
}

#[test]
fn parse_covers_every_field_with_defaults() {
    let raw = json!({
        "schedules": [{
            "id": "nightly",
            "kind": "brain",
            "target": "release-plan",
            "cron": "0 3 * * *",
            "enabled": false,
            "timezone": "+08:00",
            "overlap": "allow",
            "params": {"mode": "fixed", "objective": "cut a plan"},
            "node_id": "node-1",
        }],
        "scan_interval_secs": 17,
    });
    let cfg = SchedulesConfig::from_value(&raw);
    assert_eq!(cfg.scan_interval_secs, Some(17));
    let job = cfg.job("nightly").expect("job parsed");
    assert_eq!(job.kind, ScheduleKind::Brain);
    assert_eq!(job.target, "release-plan");
    assert_eq!(job.cron, "0 3 * * *");
    assert_eq!(job.timezone.as_deref(), Some("+08:00"));
    assert!(!job.enabled);
    assert_eq!(job.overlap, ScheduleOverlap::Allow);
    assert_eq!(job.node_id.as_deref(), Some("node-1"));
    assert_eq!(
        job.params.get("objective").and_then(Value::as_str),
        Some("cut a plan")
    );
}

#[test]
fn omitted_fields_take_their_defaults() {
    let cfg = SchedulesConfig::from_value(&json!({
        "schedules": [{"id": "minimal", "kind": "dag", "target": "etl", "cron": "* * * * *"}]
    }));
    let job = &cfg.schedules[0];
    assert_eq!(cfg.scan_interval_secs, None);
    assert!(job.enabled, "enabled defaults to true");
    assert_eq!(job.overlap, ScheduleOverlap::Skip);
    assert_eq!(job.timezone, None);
    assert_eq!(job.node_id, None);
    assert!(job.params.is_empty());
}

#[test]
fn validate_accepts_each_kind_target_contract() {
    let cases: [(&str, &str, Value); 5] = [
        ("brain", "release-plan", json!({"objective": "go"})),
        ("team", "ops-team", json!(null)),
        ("todos", "hotfix/2", json!(null)),
        ("agent", "claude-opus", json!(null)),
        ("dag", "daily-etl", json!(null)),
    ];
    for (kind, target, params) in cases {
        let mut entry = json!({"cron": "*/5 * * * *"});
        if !params.is_null() {
            entry["params"] = params;
        }
        let raw = json!({"schedules": [job_json(kind, target, entry)]});
        let cfg = SchedulesConfig::from_value(&raw);
        assert_eq!(cfg.schedules.len(), 1, "{kind} entry must parse");
        validate_all(&cfg).unwrap_or_else(|e| panic!("{kind}: {e}"));
    }
}

#[test]
fn validate_rejects_id_charset_length_and_kind_contract_violations() {
    let build = |id: &str, kind: &str, target: &str| -> SchedulesConfig {
        SchedulesConfig::from_value(&json!({
            "schedules": [job_json(kind, target, json!({
                "id": id,
                "cron": "*/5 * * * *",
            }))]
        }))
    };
    // id: bad first char, bad charset, oversized.
    for id in ["-lead", "has space", &"x".repeat(41)] {
        let err = build(id, "team", "ops").schedules[0]
            .validate()
            .unwrap_err();
        assert!(
            err.contains(&format!("schedule id {id:?}")),
            "id {id:?}: {err}"
        );
    }
    // target contracts per kind.
    assert!(build("a", "todos", "no-version-slash").schedules[0]
        .validate()
        .unwrap_err()
        .contains("template/version"));
    assert!(build("b", "brain", "bad plan id!").schedules[0]
        .validate()
        .unwrap_err()
        .contains("plan-def id"));
}

#[test]
fn brain_params_are_prechecked_against_the_fire_time_contract() {
    let build = |params: Value| -> ScheduleJob {
        let mut entry = json!({"cron": "*/5 * * * *"});
        if !params.is_null() {
            entry["params"] = params;
        }
        SchedulesConfig::from_value(&json!({
            "schedules": [job_json("brain", "plan-x", entry)]
        }))
        .schedules
        .remove(0)
    };
    assert!(build(json!(null))
        .validate()
        .unwrap_err()
        .contains("objective"));
    assert!(build(json!({"objective": ""}))
        .validate()
        .unwrap_err()
        .contains("objective"));
    assert!(build(json!({"objective": "go", "mode": "adlib"}))
        .validate()
        .unwrap_err()
        .contains("mode"));
    assert!(build(json!({"objective": "go", "inputs": [1]}))
        .validate()
        .unwrap_err()
        .contains("inputs"));
    assert!(build(json!({"objective": "go", "plan": {"id": 7}}))
        .validate()
        .unwrap_err()
        .contains("plan.id must be a string"));
    assert!(
        build(json!({"objective": "go", "plan": {"id": "p", "version": "v2"}}))
            .validate()
            .unwrap_err()
            .contains("plan.version must be a u64")
    );
    assert!(
        build(json!({"objective": "go", "plan": {"id": "p", "version": 2}}))
            .validate()
            .is_ok()
    );
}

#[test]
fn disabled_jobs_skip_cron_and_params_validation() {
    // No cron, no params, but a valid id/kind/target: validate() must pass.
    let cfg = SchedulesConfig::from_value(&json!({
        "schedules": [{"id": "off", "kind": "brain", "target": "plan-x", "enabled": false}]
    }));
    validate_all(&cfg).unwrap_or_else(|e| panic!("disabled job must skip cron/params checks: {e}"));
}

#[test]
fn from_value_is_lenient_about_invalid_entries() {
    // Structural validity is a whole-array question: one type-invalid entry
    // makes the deserializer reject the batch, which `from_value` turns into
    // a warning + empty config (fail-soft, never a panic). Id charset issues
    // are NOT structural: such entries survive and fail later in validate().
    let cfg = SchedulesConfig::from_value(&json!({
        "schedules": [
            {"id": "good", "kind": "team", "target": "ops", "cron": "* * * * *"},
            {"id": 12, "kind": "team", "target": "ops"},
        ]
    }));
    assert!(
        cfg.schedules.is_empty(),
        "a type-broken entry drops the whole batch"
    );

    let cfg = SchedulesConfig::from_value(&json!({"schedules": "not-an-array"}));
    assert!(cfg.schedules.is_empty());

    let cfg = SchedulesConfig::from_value(&json!({
        "schedules": [{"id": "bad id!", "kind": "team", "target": "ops", "cron": "* * * * *"}]
    }));
    assert_eq!(
        cfg.schedules.len(),
        1,
        "id validation happens later, not at parse"
    );
}

#[test]
fn domain_file_routing_loads_project_first() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join(".opencoder")).unwrap();

    let global = dir.path().join("global-home");
    std::fs::create_dir_all(global.join(".opencoder")).unwrap();
    let _guard = crate::config::env::scoped_config_home(global);
    let global_file = super::super::domain::global_domain_path("schedules").unwrap();
    std::fs::write(
        &global_file,
        json!({"schedules": [{"id": "from-global", "kind": "team", "target": "g",
                               "cron": "* * * * *", "enabled": true}]})
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        schedules_path(&work),
        Some(global_file.clone()),
        "project file missing yet"
    );
    // No project file yet: the global one is the effective source.
    let cfg = load_schedules(&work);
    assert_eq!(cfg.schedules.len(), 1);
    assert_eq!(cfg.schedules[0].id, "from-global");

    // A project file shadows the global one entirely.
    std::fs::write(
        work.join(".opencoder").join("schedules.json"),
        json!({"schedules": [{"id": "from-project", "kind": "todos", "target": "t/1",
                               "cron": "0 * * * *", "enabled": true}]})
        .to_string(),
    )
    .unwrap();
    let cfg = load_schedules(&work);
    assert_eq!(cfg.schedules.len(), 1);
    assert_eq!(cfg.schedules[0].id, "from-project");
    assert_eq!(
        schedules_path(&work),
        Some(work.join(".opencoder").join("schedules.json"))
    );
}

#[test]
fn execution_kind_mapping_covers_the_five_kinds() {
    for (kind, want) in [
        (ScheduleKind::Brain, ExecutionKind::Brain),
        (ScheduleKind::Team, ExecutionKind::Team),
        (ScheduleKind::Todos, ExecutionKind::Todos),
        (ScheduleKind::Agent, ExecutionKind::Agent),
        (ScheduleKind::Dag, ExecutionKind::Dag),
    ] {
        assert_eq!(kind.execution_kind(), want);
    }
}

#[test]
fn merge_replaces_the_schedule_array_and_keeps_id_order() {
    let mut cfg = SchedulesConfig::from_value(&json!({
        "schedules": [{"id": "old", "kind": "team", "target": "ops", "cron": "* * * * *"}],
        "scan_interval_secs": Some(999),
    }));
    let entries = json!({
        "schedules": [
            {"id": "zeta", "kind": "team", "target": "ops", "cron": "* * * * *"},
            {"id": "alpha", "kind": "dag", "target": "etl", "cron": "* * * * *"},
        ],
        "scan_interval_secs": 30,
    })
    .as_object()
    .unwrap()
    .clone();
    merge(&mut cfg, &entries);
    // Array replaced wholesale; id order preserved as written.
    let ids: Vec<&str> = cfg.schedules.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(ids, ["zeta", "alpha"]);
    assert_eq!(cfg.scan_interval_secs, Some(30));
    // An unrelated stale job must be gone.
    assert!(cfg.job("old").is_none());

    // A batch the deserializer rejects leaves the previous jobs untouched.
    let broken = json!({"schedules": [{"id": 5}]})
        .as_object()
        .unwrap()
        .clone();
    merge(&mut cfg, &broken);
    assert_eq!(cfg.schedules.len(), 2);
    assert_eq!(cfg.scan_interval_secs, Some(30));
}

#[test]
fn merge_ignores_nonpositive_scan_interval() {
    let mut cfg = SchedulesConfig {
        scan_interval_secs: Some(99),
        ..SchedulesConfig::default()
    };
    let entries = json!({
        "schedules": [{"id": "a", "kind": "team", "target": "ops", "cron": "* * * * *"}],
        "scan_interval_secs": 0,
    })
    .as_object()
    .unwrap()
    .clone();
    merge(&mut cfg, &entries);
    // 0 warns and falls back to the default interval; never an error.
    assert_eq!(cfg.scan_interval_secs, None);
    assert_eq!(cfg.schedules.len(), 1);
}
