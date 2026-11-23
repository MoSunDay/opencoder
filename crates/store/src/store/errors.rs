//! Error classification for `Store` callers that recover from specific
//! storage failures. Backend-specific downcasts (`turso::Error`) stay in
//! this crate so upper layers never depend on the storage engine directly.

/// Whether the error chain reports a foreign-key violation.
///
/// On the message-append path this means `messages.session_id` references a
/// `sessions` row the storage layer cannot find: either another process
/// sharing the project database deleted the session (cross-process
/// `/task clear all`, web clear, `dialogs_clear`, `session delete`), or a
/// storage-level desync hid the row from the FK probe. Immediate FKs fail
/// the INSERT itself, so the statement's transaction has already rolled
/// back and the write can be retried after the parent row is recreated.
pub fn is_foreign_key_violation(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<turso::Error>().is_some_and(|error| {
            matches!(error, turso::Error::Constraint(message) if message.contains("FOREIGN KEY"))
        })
    })
}

/// Whether the error chain reports storage corruption (`SQLITE_CORRUPT`):
/// unreadable rows/pages, inconsistent overflow chains, or a desynced index.
///
/// Full-table statements fail as a whole on such a database even when only
/// one row is unreadable, so callers that can work around the damaged row
/// (listing page by page, best-effort backfills) branch on this.
pub fn is_storage_corruption(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<turso::Error>()
            .is_some_and(is_corruption_error)
    })
}

/// Whether the error chain reports storage the database worker could not
/// read: either reported as corruption ([`is_storage_corruption`]) or as a
/// worker panic.
///
/// Turso 0.8.3 asserts on damaged b-tree pages of a shape it does not
/// expect instead of returning `SQLITE_CORRUPT`; the database executor
/// surfaces the panic as a `JoinError`. Callers that can route around the
/// damaged pages treat both alike.
pub fn is_unreadable_storage(error: &anyhow::Error) -> bool {
    is_storage_corruption(error)
        || error
            .chain()
            .any(|cause| cause.downcast_ref::<tokio::task::JoinError>().is_some())
}

/// [`is_storage_corruption`] for a raw driver error, before any context is
/// attached (statement results handled right where they are produced).
pub fn is_corruption_error(error: &turso::Error) -> bool {
    matches!(error, turso::Error::Corrupt(_))
}

#[cfg(test)]
mod tests {
    use super::{
        is_corruption_error, is_foreign_key_violation, is_storage_corruption, is_unreadable_storage,
    };

    #[test]
    fn foreign_key_violation_is_detected_through_anyhow_context() {
        let error = anyhow::Error::new(turso::Error::Constraint(
            "FOREIGN KEY constraint failed".into(),
        ))
        .context("insert message in tx");
        assert!(is_foreign_key_violation(&error));
    }

    #[test]
    fn other_constraints_and_corruption_are_not_foreign_key_violations() {
        let unique = anyhow::Error::new(turso::Error::Constraint(
            "UNIQUE constraint failed: sessions.id".into(),
        ))
        .context("insert session");
        assert!(!is_foreign_key_violation(&unique));
        let corrupt = anyhow::Error::new(turso::Error::Corrupt(
            "IdxDelete: no matching index entry found".into(),
        ))
        .context("touch session activity in tx");
        assert!(!is_foreign_key_violation(&corrupt));
        assert!(is_storage_corruption(&corrupt));
        assert!(!is_storage_corruption(&unique));
        assert!(!is_storage_corruption(&anyhow::anyhow!("plain error")));
        assert!(is_unreadable_storage(&corrupt));
        assert!(!is_unreadable_storage(&unique));
    }

    #[test]
    fn worker_panic_is_unreadable_storage() {
        let panic = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                tokio::spawn(async {
                    panic!("b-tree seek on a damaged interior page");
                })
                .await
            })
            .unwrap_err();
        let error = anyhow::Error::new(panic).context("database operation panicked");
        assert!(is_unreadable_storage(&error));
        assert!(!is_storage_corruption(&error));
        assert!(!is_foreign_key_violation(&error));
        assert!(is_corruption_error(&turso::Error::Corrupt(
            "inconsistent overflow chain observed during payload read".into()
        )));
        assert!(!is_corruption_error(&turso::Error::Constraint("fk".into())));
    }
}
