//! Input lifecycle forwarding through `TsMirrorStore`.
//!
//! The `Store` trait defaults for `mark_inputs_recorded`, `unpromote_inputs`,
//! `recover_orphan_inputs` and `orphan_inputs` are no-op stubs for test fakes.
//! A production wrapper that inherits them silently drops the runner's
//! consumption bookkeeping in the underlying store: a consumed input stays
//! promoted-but-unrecorded, so a later `session export` (its own store handle)
//! carries it as an orphan and `session import` re-delivers it; an
//! error-recovery unpromote or a run-entry orphan recovery is lost the same
//! way. These cases drive every lifecycle call through the wrapper and assert
//! the state the underlying store and an imported copy actually see.
//!
//! Split from `ts_mirror_tests.rs` (test-file size limit); its fixtures are
//! reused through `super::*`.

use super::*;
use opencoder_store::{export_bundle, import_bundle, Delivery, SessionInput};

/// A pending input row as the composer admits it (store assigns the seqs).
fn input(session: &str, id: &str, delivery: Delivery) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.into(),
        session_id: session.into(),
        delivery,
        prompt: format!("p-{id}"),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

fn ids(inputs: &[SessionInput]) -> Vec<&str> {
    inputs.iter().map(|input| input.id.as_str()).collect()
}

async fn pending_ids(store: &dyn Store, session: &str, delivery: Delivery) -> Vec<String> {
    store
        .pending_inputs(session, delivery)
        .await
        .unwrap()
        .into_iter()
        .map(|input| input.id)
        .collect()
}

/// consume -> export -> import: a promoted-and-recorded steer row is never
/// exported, so importing the bundle cannot re-deliver it. The still-pending
/// queue row and the promoted-but-unrecorded orphan (both owed to the next
/// drain) do ride along, which proves the export read the underlying store's
/// real input states instead of an empty default.
#[tokio::test]
async fn consumed_inputs_are_not_exported_or_re_delivered() {
    let (mirror, _tmp, _workdir) = fixture().await;
    let id = "01CONSUMED";
    mirror.create_session(&ts_meta(id)).await.unwrap();
    for (name, delivery) in [
        ("consumed", Delivery::Steer),
        ("orphan", Delivery::Steer),
        ("queued", Delivery::Queue),
    ] {
        mirror
            .admit_input(&input(id, name, delivery))
            .await
            .unwrap();
    }
    let promoted = mirror.promote_inputs(id, 2, Delivery::Steer).await.unwrap();
    assert_eq!(promoted.len(), 2, "both steer rows are promoted");
    // The runner consumed the first steer row into the transcript.
    mirror
        .mark_inputs_recorded(id, &[promoted[0]])
        .await
        .unwrap();

    // The record reached the underlying store: a handle opened on it the way
    // `session export` opens one sees only the unconsumed orphan.
    assert_eq!(
        ids(&mirror.inner.orphan_inputs(id).await.unwrap()),
        ["orphan"],
        "the consumed row must not look orphaned in the underlying store"
    );

    let bundle = export_bundle(&mirror, id).await.unwrap();
    assert_eq!(
        ids(&bundle.inputs),
        ["orphan", "queued"],
        "consumed rows are never exported; pending and orphaned rows are"
    );

    let target = TursoStore::open_memory().await.unwrap();
    let imported = import_bundle(&target, &bundle, None).await.unwrap();
    assert_eq!(imported, id, "import keeps the session id");
    assert_eq!(
        pending_ids(&target, id, Delivery::Steer).await,
        ["orphan"],
        "only the orphan is re-admitted as a pending steer"
    );
    assert_eq!(
        pending_ids(&target, id, Delivery::Queue).await,
        ["queued"],
        "only the unconsumed queue row is re-admitted"
    );
    assert!(
        !bundle.inputs.iter().any(|input| input.id == "consumed"),
        "no bundle row can re-deliver the consumed prompt"
    );
}

/// Error recovery and run-entry recovery through the wrapper: an unpromoted
/// steer row is pending again and the remaining orphan is counted and handed
/// back to the next drain — in the underlying store, not only in the
/// wrapper's own return values.
#[tokio::test]
async fn unpromote_and_orphan_recovery_reach_the_underlying_store() {
    let (mirror, _tmp, _workdir) = fixture().await;
    let id = "01RECOVER";
    mirror.create_session(&ts_meta(id)).await.unwrap();
    for name in ["failed", "orphan"] {
        mirror
            .admit_input(&input(id, name, Delivery::Steer))
            .await
            .unwrap();
    }
    let promoted = mirror.promote_inputs(id, 2, Delivery::Steer).await.unwrap();
    assert_eq!(promoted.len(), 2, "both steer rows are promoted");
    assert_eq!(
        ids(&mirror.orphan_inputs(id).await.unwrap()),
        ["failed", "orphan"],
        "promoted-but-unrecorded rows are visible through the wrapper"
    );

    // The batch failed on its first item: the runner unpromotes it so the
    // next run picks it up again.
    mirror.unpromote_inputs(id, &[promoted[0]]).await.unwrap();
    assert_eq!(
        ids(&mirror.pending_inputs(id, Delivery::Steer).await.unwrap()),
        ["failed"],
        "the unpromoted row is pending again through the wrapper"
    );
    assert_eq!(
        pending_ids(mirror.inner.as_ref(), id, Delivery::Steer).await,
        ["failed"],
        "the unpromote reached the underlying store"
    );
    assert_eq!(ids(&mirror.orphan_inputs(id).await.unwrap()), ["orphan"]);

    // Run entry: the remaining orphan is flipped back to pending, idempotently.
    assert_eq!(
        mirror.recover_orphan_inputs(id).await.unwrap(),
        1,
        "exactly the promoted-but-unrecorded row is recovered"
    );
    assert_eq!(
        pending_ids(mirror.inner.as_ref(), id, Delivery::Steer).await,
        ["failed", "orphan"],
        "the recovered row is pending in the underlying store, in drain order"
    );
    assert!(
        mirror.inner.orphan_inputs(id).await.unwrap().is_empty(),
        "no row is left promoted-but-unrecorded"
    );
    assert_eq!(mirror.recover_orphan_inputs(id).await.unwrap(), 0);
}
