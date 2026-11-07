//! Unit tests for path defaults, marker state and the decision table.
//! Strict row decoding and bundle assembly have their own files; end-to-end
//! migration against a synthetic legacy store lives in
//! `tests/legacy_migration.rs`.

mod assembly_tests;
mod parse_tests;

use std::path::{Path, PathBuf};

use super::rows::LegacySession;

fn meta(id: &str, created_at: i64) -> opencoder_store::SessionMeta {
    opencoder_store::SessionMeta {
        kind: None,
        id: id.to_string(),
        title: Some(format!("title {id}")),
        agent: Some("plan".into()),
        model: Some("prov/m".into()),
        autopilot_mode: None,
        workdir_hash: Some("hash".into()),
        created_at,
        updated_at: created_at,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
    }
}

fn session(id: &str, created_at: i64) -> LegacySession {
    LegacySession {
        meta: meta(id, created_at),
    }
}

fn text_message(id: &str, text: &str) -> opencoder_core::Message {
    opencoder_core::Message::user(id, text)
}

#[test]
fn default_paths_follow_the_contract() {
    let data_dir = PathBuf::from("/data/abc");
    let opts = super::MigrateOptions::defaults(&data_dir);
    assert_eq!(opts.source, PathBuf::from("/data/abc/opencoder.db"));
    assert_eq!(
        opts.destination,
        opencoder_core::session_store_rdb_dir(&data_dir)
    );
    assert_eq!(
        opts.backup,
        PathBuf::from("/data/abc/opencoder.db.pre-rdb.bak")
    );
    assert!(!opts.dry_run);
    assert_eq!(
        super::marker_path(&opts.source),
        PathBuf::from("/data/abc/opencoder.db.migrated.json")
    );
}

#[test]
fn marker_roundtrip_and_invalid_states() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("opencoder.db.migrated.json");
    assert_eq!(
        super::read_marker(&path).unwrap(),
        super::MarkerState::Absent
    );

    let opts = super::MigrateOptions::defaults(tmp.path());
    let counts = super::MigrationCounts {
        roots: 1,
        sessions: 2,
        messages: 3,
        events: 4,
        inputs: 5,
        subagent_tasks: 6,
    };
    super::write_marker(&path, &opts, counts).unwrap();
    match super::read_marker(&path).unwrap() {
        super::MarkerState::Valid(m) => {
            assert_eq!(m.format, 1);
            assert_eq!(m.counts, counts);
            assert_eq!(m.source, opts.source.display().to_string());
            assert_eq!(m.destination, opts.destination.display().to_string());
            assert!(m.migrated_at > 0, "epoch ms timestamp");
            // Non-sensitive: the marker body carries no session content.
            let body = std::fs::read_to_string(&path).unwrap();
            assert!(!body.contains("title "), "no session text leaked");
        }
        other => panic!("expected valid marker, got {other:?}"),
    }

    std::fs::write(&path, b"{ not json").unwrap();
    assert!(matches!(
        super::read_marker(&path).unwrap(),
        super::MarkerState::Invalid(_)
    ));

    let wrong_format = serde_json::json!({
        "format": 99,
        "source": "a", "destination": "b", "backup": "c",
        "counts": {"roots": 0, "sessions": 0, "messages": 0, "events": 0,
                   "inputs": 0, "subagent_tasks": 0},
        "migrated_at": 1
    });
    std::fs::write(&path, wrong_format.to_string()).unwrap();
    match super::read_marker(&path).unwrap() {
        super::MarkerState::Invalid(reason) => {
            assert!(
                reason.contains("format"),
                "reason mentions format: {reason}"
            );
        }
        other => panic!("expected invalid marker, got {other:?}"),
    }
}

#[test]
fn decision_table_for_valid_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let destination = tmp.path().join("opencoder.db.rdb");
    std::fs::create_dir_all(&destination).unwrap();
    let marker = super::MigrationMarker {
        format: 1,
        source: "s".into(),
        destination: destination.display().to_string(),
        backup: "b".into(),
        counts: Default::default(),
        migrated_at: 1,
    };
    assert!(matches!(
        super::decide(Some(&marker), &destination),
        super::MigrationDecision::AlreadyMigrated(_)
    ));
    // Recorded destination differs from the resolved one → stale.
    let elsewhere = tmp.path().join("other.rdb");
    assert!(matches!(
        super::decide(Some(&marker), &elsewhere),
        super::MigrationDecision::StaleMarker(_)
    ));
    // Destination directory missing → stale, not already-migrated.
    std::fs::remove_dir_all(&destination).unwrap();
    assert!(matches!(
        super::decide(Some(&marker), &destination),
        super::MigrationDecision::StaleMarker(_)
    ));
    assert!(matches!(
        super::decide(None, &destination),
        super::MigrationDecision::Proceed
    ));
}

#[test]
fn render_report_is_faithful_to_the_run() {
    let opts = super::MigrateOptions::defaults(Path::new("/data/abc"));
    let mut report = super::MigrationReport {
        dry_run: true,
        roots_total: 2,
        counts: super::MigrationCounts {
            sessions: 3,
            messages: 4,
            events: 5,
            inputs: 6,
            subagent_tasks: 7,
            ..Default::default()
        },
        ..Default::default()
    };
    let text = super::render_report(&report, &opts);
    assert!(text.contains("dry run"));
    assert!(text.contains("2 would migrate"));
    assert!(!text.contains("backup"), "dry run reports no backup");

    report = super::MigrationReport {
        dry_run: false,
        roots_total: 2,
        roots_imported: 1,
        roots_skipped: 1,
        backup: Some(opts.backup.clone()),
        backup_created: true,
        marker: Some(super::marker_path(&opts.source)),
        ..report.clone()
    };
    let text = super::render_report(&report, &opts);
    assert!(text.contains("1 imported, 1 already present, 2 total"));
    assert!(text.contains("created"));
    assert!(text.contains("migrated.json"));
}
