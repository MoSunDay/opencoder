use super::*;

fn require_harness_write(session: &mut SessionState, dir: &Path) {
    session.harness.resource_root = Some(dir.join("lazy-resources"));
}

#[tokio::test]
async fn lazy_shared_selection_rolls_back_a_row_created_by_the_failed_operation() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = state(
        "lazy-shared-rollback",
        "build",
        config("provider/base", "provider/act", "provider/plan"),
        store.clone(),
        dir.path(),
    );
    require_harness_write(&mut session, dir.path());
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_lazy_harness BEFORE UPDATE OF harness_runtime ON sessions BEGIN SELECT RAISE(FAIL, 'injected lazy harness failure'); END",
        (),
    )
    .await
    .unwrap();

    let error = session
        .set_shared_model_override("provider/rejected-shared")
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected lazy harness failure"));
    assert!(store
        .get_session("lazy-shared-rollback")
        .await
        .unwrap()
        .is_none());
    assert_eq!(session.model_override(ConfigMode::Shared), None);
    assert_eq!(session.effective_model(), "provider/base");
}

#[tokio::test]
async fn lazy_cleanup_failure_reports_residue_instead_of_returning_success() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = state(
        "lazy-residue",
        "build",
        config("provider/base", "provider/act", "provider/plan"),
        store.clone(),
        dir.path(),
    );
    require_harness_write(&mut session, dir.path());
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_residue_harness BEFORE UPDATE OF harness_runtime ON sessions BEGIN SELECT RAISE(FAIL, 'injected harness residue failure'); END",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "CREATE TRIGGER reject_residue_delete BEFORE DELETE ON sessions BEGIN SELECT RAISE(FAIL, 'injected delete residue failure'); END",
        (),
    )
    .await
    .unwrap();

    let error = session
        .set_shared_model_override("provider/rejected-shared")
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("injected harness residue failure"),
        "{message}"
    );
    assert!(
        message.contains("injected delete residue failure"),
        "{message}"
    );
    assert!(message.contains("residue"), "{message}");
    assert!(store.get_session("lazy-residue").await.unwrap().is_some());
    assert_eq!(session.model_override(ConfigMode::Shared), None);
}

#[tokio::test]
async fn lazy_selection_failure_never_deletes_a_preexisting_row() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let original = SessionModelOverrides {
        act: Some("provider/original-act".into()),
        ..Default::default()
    };
    store
        .create_session_with_model_overrides(
            &meta("lazy-preexisting", "provider/original-base"),
            Some(&original),
        )
        .await
        .unwrap();
    // Deliberately leave the handle lazy even though the row already exists.
    let mut session = state(
        "lazy-preexisting",
        "act",
        config("provider/base", "provider/act", "provider/plan"),
        store.clone(),
        dir.path(),
    );
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_preexisting_override BEFORE UPDATE OF model_overrides_json ON sessions BEGIN SELECT RAISE(FAIL, 'injected pre-existing update failure'); END",
        (),
    )
    .await
    .unwrap();

    session
        .set_mode_model_override(ConfigMode::Act, "provider/rejected-act")
        .await
        .unwrap_err();
    let row = store.get_session("lazy-preexisting").await.unwrap();
    assert_eq!(
        row.unwrap().model.as_deref(),
        Some("provider/original-base")
    );
    assert_eq!(
        store
            .get_session_model_overrides("lazy-preexisting")
            .await
            .unwrap(),
        Some(original)
    );
    assert_eq!(session.model_override(ConfigMode::Act), None);
    assert_eq!(session.effective_model(), "provider/act");
}
