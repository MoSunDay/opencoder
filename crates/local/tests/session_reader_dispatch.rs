//! `session list|show` open the store read-only: readers take no lease, so
//! they work beside any live writer, never block or displace one, and report
//! "nothing" without creating the database. The multi-writer contention and
//! maintenance-lease suites live in the database crate's
//! `tests/maintenance_lock.rs`, `tests/busy_contention.rs` and
//! `tests/multi_writer_soak.rs`.
use clap::Parser;
use opencoder_local::session_cmd::session_dispatch;
use opencoder_local::{Cli, SessionSub};
use opencoder_store::{read_bundle, Delivery, SessionInput, SessionMeta, Store, TursoStore};
use std::path::Path;

fn cli(workdir: &Path) -> Cli {
    Cli::parse_from([
        "opencoder".to_string(),
        "--workdir".to_string(),
        workdir.to_string_lossy().into_owned(),
        "session".to_string(),
        "list".to_string(),
    ])
}

/// The per-workdir database `session` resolves for `workdir`.
fn database_for(workdir: &Path) -> std::path::PathBuf {
    opencoder_core::data_dir_for(workdir).join("opencoder.db")
}

#[tokio::test]
async fn read_only_session_commands_report_nothing_without_creating_a_database() {
    let workdir = tempfile::tempdir().unwrap();
    let cli = cli(workdir.path());
    let database = database_for(workdir.path());
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
    assert!(!database.exists());

    session_dispatch(&SessionSub::List, &cli).await.unwrap();
    let missing = session_dispatch(
        &SessionSub::Show {
            id: "absent".into(),
            json: false,
        },
        &cli,
    )
    .await
    .unwrap_err();
    assert!(
        missing.to_string().contains("session not found"),
        "{missing}"
    );
    assert!(
        !database.exists(),
        "a read-only command must never create the database"
    );
}

#[tokio::test]
async fn read_only_session_commands_work_beside_a_live_writer() {
    let workdir = tempfile::tempdir().unwrap();
    let cli = cli(workdir.path());
    let database = database_for(workdir.path());
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    // A live writer owns the database the way a running TUI does.
    let writer = TursoStore::open(&database).await.unwrap();
    writer
        .create_session(&SessionMeta {
            id: "beside-writer".into(),
            title: Some("listed".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    session_dispatch(&SessionSub::List, &cli).await.unwrap();
    session_dispatch(
        &SessionSub::Show {
            id: "beside-writer".into(),
            json: false,
        },
        &cli,
    )
    .await
    .unwrap();
    // The writer is undisturbed and keeps writing.
    writer
        .create_session(&SessionMeta {
            id: "second".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        writer
            .list_sessions(&Default::default())
            .await
            .unwrap()
            .len(),
        2,
        "the read-only commands changed nothing"
    );
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}

/// `session export` shares the read-only opening, so the bundle it writes
/// beside a live writer carries that writer's orphaned input (promoted but
/// never recorded) without recovering it in place.
#[tokio::test]
async fn read_only_session_export_beside_a_live_writer_carries_orphaned_inputs() {
    let workdir = tempfile::tempdir().unwrap();
    let cli = cli(workdir.path());
    let database = database_for(workdir.path());
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let writer = TursoStore::open(&database).await.unwrap();
    writer
        .create_session(&SessionMeta {
            id: "exported".into(),
            title: Some("exported".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    writer
        .admit_input(&SessionInput {
            seq: None,
            id: "orphan".into(),
            session_id: "exported".into(),
            delivery: Delivery::Steer,
            prompt: "still owed".into(),
            images: Vec::new(),
            display_text: None,
            admitted_seq: 0,
            promoted_seq: None,
        })
        .await
        .unwrap();
    assert_eq!(
        writer
            .promote_inputs("exported", 1, Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1,
        "promoted, never recorded: pending_inputs cannot see it"
    );

    let out = workdir.path().join("exported.opencoder");
    session_dispatch(
        &SessionSub::Export {
            id: "exported".into(),
            out: Some(out.clone()),
        },
        &cli,
    )
    .await
    .unwrap();

    let bundle = read_bundle(&mut std::fs::File::open(&out).unwrap()).unwrap();
    assert_eq!(bundle.meta.id, "exported");
    assert_eq!(
        bundle
            .inputs
            .iter()
            .map(|input| input.id.as_str())
            .collect::<Vec<_>>(),
        ["orphan"],
        "the exported bundle must carry the orphaned input"
    );
    // The export wrote nothing: the writer still holds the promoted orphan.
    assert_eq!(
        writer.recover_orphan_inputs("exported").await.unwrap(),
        1,
        "export must leave recovery to the writer"
    );
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}
