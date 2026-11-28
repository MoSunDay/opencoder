//! `session list|show|export` open the store read-only, and a reader never
//! migrates: on a database older than this build the read fails, and the
//! command must say so with the remedy instead of surfacing the engine's raw
//! query error. Other failures (a missing session on a current schema) keep
//! their own wording.
mod support;

use clap::Parser;
use opencoder_local::session_cmd::session_dispatch;
use opencoder_local::{Cli, SessionSub};
use opencoder_store::{Store, TursoStore};
use std::path::{Path, PathBuf};

fn cli(workdir: &Path) -> Cli {
    Cli::parse_from([
        "opencoder".to_string(),
        "--workdir".to_string(),
        workdir.to_string_lossy().into_owned(),
        "session".to_string(),
        "list".to_string(),
    ])
}

/// The raw error this build's reader produces on the old schema for one read,
/// captured straight from the store so the test does not hard-code engine
/// wording. Each read is its own statement, so its error is its own.
async fn raw_reader_error<F, Fut>(database: &Path, read: F) -> String
where
    F: FnOnce(TursoStore) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let reader = TursoStore::open_reader(database).await.unwrap();
    read(reader)
        .await
        .expect_err("the old schema is not readable through a reader")
        .to_string()
}

/// The listing read of `session list`.
async fn list_all(store: TursoStore) -> anyhow::Result<()> {
    store.list_sessions(&Default::default()).await.map(|_| ())
}

/// The first read of `session show` and `session export`: the session meta.
async fn fetch_meta(store: TursoStore, id: &'static str) -> anyhow::Result<()> {
    store.get_session(id).await.map(|_| ())
}

fn assert_actionable(error: anyhow::Error, database: &Path, raw: &str) {
    let top = error.to_string();
    assert!(top.contains(&database.display().to_string()), "{error:#}");
    assert!(top.contains("has schema version 3"), "{error:#}");
    assert!(
        top.contains(&format!(
            "older than this build reads (v{})",
            opencoder_store::turso_store::schema_watermark()
        )),
        "{error:#}"
    );
    assert!(
        top.contains("read-only commands never migrate it"),
        "{error:#}"
    );
    assert!(top.contains("`opencoder tui`"), "{error:#}");
    assert!(
        format!("{error:#}").contains(raw),
        "the engine's own error stays as the cause: {error:#} (raw reader error: {raw})"
    );
}

#[tokio::test]
async fn read_only_session_commands_explain_an_old_schema_instead_of_the_raw_error() {
    let workdir = tempfile::tempdir().unwrap();
    let cli = cli(workdir.path());
    let database = support::old_schema_store(workdir.path()).await;
    let listing = raw_reader_error(&database, list_all).await;
    let meta = raw_reader_error(&database, |store| fetch_meta(store, "OLD1")).await;
    assert_ne!(
        listing, meta,
        "fixture sanity: each read is its own statement"
    );
    for raw in [&listing, &meta] {
        assert!(
            !raw.contains("schema version"),
            "fixture sanity, the raw error is the engine's: {raw}"
        );
    }

    assert_actionable(
        session_dispatch(&SessionSub::List, &cli).await.unwrap_err(),
        &database,
        &listing,
    );
    for sub in [
        SessionSub::Show {
            id: "OLD1".into(),
            json: false,
        },
        SessionSub::Export {
            id: "OLD1".into(),
            out: None,
        },
    ] {
        assert_actionable(
            session_dispatch(&sub, &cli).await.unwrap_err(),
            &database,
            &meta,
        );
    }

    // The remedy is real: one writer open migrates the store, after which
    // the same read-only command lists the session.
    let writer = TursoStore::open(&database).await.unwrap();
    writer
        .create_session(&opencoder_store::SessionMeta {
            id: "fresh".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    session_dispatch(&SessionSub::List, &cli).await.unwrap();
    let reader = TursoStore::open_reader(&database).await.unwrap();
    assert!(reader.get_session("OLD1").await.unwrap().is_some());
    drop(reader);
    drop(writer);
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}

#[tokio::test]
async fn missing_session_on_a_current_schema_keeps_its_own_wording() {
    let workdir = tempfile::tempdir().unwrap();
    let cli = cli(workdir.path());
    let database: PathBuf = support::database_for(workdir.path());
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let writer = TursoStore::open(&database).await.unwrap();
    drop(writer);

    let error = session_dispatch(
        &SessionSub::Show {
            id: "absent".into(),
            json: false,
        },
        &cli,
    )
    .await
    .unwrap_err();
    let chain = format!("{error:#}");
    assert!(chain.contains("session not found: absent"), "{chain}");
    assert!(
        !chain.contains("older than this build"),
        "a current schema is never reported as old: {chain}"
    );
    std::fs::remove_dir_all(database.parent().unwrap()).ok();
}
