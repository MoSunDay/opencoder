//! `session_inputs.recorded` lifecycle tests for the libsql-backed Store.
//!
//! Covers the recorded state machine (admit → promote → mark_recorded), the
//! promote-resets-recorded invariant, orphan recovery (promoted but never
//! recorded rows flipped back to pending), and the v9→v10 migration backfill
//! that treats pre-existing promoted rows as consumed. Split out of
//! `inputs_integration.rs` to keep each file focused and under the line-count
//! limit. Runs against a real on-disk libsql file (tempdir).

use libsql::{params, Connection};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
}

async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),

        autopilot_mode: None,
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        kind: None,
    };
    store.create_session(&meta).await.unwrap();
}

fn mk_input(sid: &str, id: &str, delivery: Delivery) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.to_string(),
        session_id: sid.into(),
        delivery,
        prompt: format!("p-{id}"),
        images: Vec::new(),
        display_text: None,
        // Value is ignored: the store recomputes admitted_seq per session.
        admitted_seq: 0,
        promoted_seq: None,
    }
}

// Raw `(promoted_seq, recorded)` state of an input row, bypassing the Store
// API so the tests assert what actually landed in the table.
async fn input_state(conn: &Connection, seq: i64) -> (Option<i64>, i64) {
    let stmt = conn
        .prepare("SELECT promoted_seq, recorded FROM session_inputs WHERE seq = ?")
        .await
        .unwrap();
    let mut rows = stmt.query(params![seq]).await.unwrap();
    let r = rows.next().await.unwrap().expect("input row exists");
    (r.get(0).unwrap(), r.get(1).unwrap())
}

// The full recorded state machine: a row starts pending with recorded=0,
// promotion keeps recorded=0 (not yet consumed), and mark_inputs_recorded
// flips it to 1 once the input is durably in the transcript. The row stays
// invisible to `pending_inputs` from promotion onward.

// Promoting resets recorded=0: a row that was previously promoted AND
// recorded, then unpromoted (error recovery), must not carry a stale
// recorded=1 into its next promotion — otherwise recover_orphan_inputs
// would silently skip it after a later crash.

// recover_orphan_inputs flips exactly the promoted-but-unrecorded rows back
// to pending: an orphan (promoted, crash before consume) is recovered and
// visible to pending_inputs again, while a properly recorded promoted row
// and a never-promoted pending row are untouched. Idempotent.

// v9→v10 migration backfill: simulate a legacy v9 database (version row
// rewound to 9, a promoted input reset to the pre-column recorded=0 state),
// reopen through `LibsqlStore::open` so bootstrap runs the migration, then
// assert the version bumps to 10 and the pre-existing promoted row is
// backfilled to recorded=1 (historical audit rows count as consumed) while a
// pending row keeps recorded=0.

// Queue-drain invariant: a row already consumed into the transcript
// (`recorded = 1`) must NEVER be re-claimed by `claim_next_queue`, even when
// an error-recovery `unpromote_inputs` flipped it back to pending (that path
// only clears `promoted_seq`, leaving `recorded` as-is). Re-serving it would
// duplicate the prompt in the transcript.
//
// Repro: enqueue two queue rows → claim row 1 → mark it recorded → unpromote
// it (recovery path) → the next claim must serve row 2, never row 1 again.
// Pre-fix this is red: the claim SELECT only checks `promoted_seq IS NULL`,
// so row 1 (pending + recorded=1) is re-served first.

// Same invariant for `promote_next_queued`: a consumed (recorded=1) row that
// error recovery returned to pending must be skipped, not re-promoted.
// Pre-fix this is red: it re-promotes row 1 and returns its seq.

#[path = "inputs_recorded/suite_1.rs"]
mod suite_1;
