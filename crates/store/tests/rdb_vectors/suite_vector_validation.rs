//! Strict validation of embedding payloads on every write path, plus
//! search's skip-don't-fail handling of corrupt stored rows: non-positive
//! dims, wrong-length blobs and non-finite values are rejected before any
//! batch stages; rows already corrupted in storage are skipped, never
//! scored, never fatal.

use super::*;
use opencoder_store::rdb_store::engine::RdbDb;
use opencoder_store::rdb_store::execution::brain;
use opencoder_store::rdb_store::tx;

/// The shared write gate: non-positive dims, blobs that are not exactly
/// `dim * 4` bytes and non-finite values are all rejected BEFORE any batch
/// is staged — nothing commits, and valid payloads keep flowing through
/// the very same paths.
#[tokio::test]
async fn invalid_vector_payloads_are_rejected_and_commit_nothing() {
    let store = mem().await;
    store
        .create_brain_capability(&cap("cap-1", "goal", 100), &[])
        .await
        .unwrap();

    // Non-positive dims are not vectors.
    for dim in [0, -4] {
        assert!(
            store
                .create_brain_capability_with_vector(
                    &cap("cap-bad", "goal", 200),
                    &[],
                    &vec_bytes(dim, "emb", le(&[1.0, 0.0, 0.0, 0.0]), 1),
                )
                .await
                .is_err(),
            "dim {dim} must be rejected"
        );
    }
    // The blob must be exactly dim * 4 bytes: too short, torn tail and
    // too long all fail the exact-length contract.
    let four = le(&[1.0, 0.0, 0.0, 0.0]);
    for emb in [
        le(&[1.0, 0.0, 0.0]),                   // 3 values, dim 4
        four[..four.len() - 1].to_vec(),        // torn tail byte
        [four.as_slice(), &four[..4]].concat(), // 5 values, dim 4
    ] {
        assert!(
            store
                .create_brain_capability_with_vector(
                    &cap("cap-bad", "goal", 200),
                    &[],
                    &vec_bytes(4, "emb", emb.clone(), 1),
                )
                .await
                .is_err(),
            "blob of {} bytes must not pass as dim 4",
            emb.len()
        );
    }
    // Non-finite values are rejected even at the exact byte length.
    for emb in [le(&[f32::NAN, 0.0, 0.0, 0.0]), le(&[0.0, f32::INFINITY])] {
        let dim = emb.len() as i64 / 4;
        assert!(
            store
                .create_brain_capability_with_vector(
                    &cap("cap-bad", "goal", 200),
                    &[],
                    &vec_bytes(dim, "emb", emb.clone(), 1),
                )
                .await
                .is_err(),
            "non-finite values must be rejected"
        );
    }
    // Every rejection happened before the batch: no capability row, no
    // inputs and no vector row were committed for the rejected payload.
    assert!(store
        .get_brain_capability("cap-bad")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap()
        .is_empty());

    // The guarded combined update enforces the same gate on a live row.
    store
        .upsert_brain_vector("cap-1", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();
    let detail = store.get_brain_capability("cap-1").await.unwrap().unwrap();
    let mut updated = cap("cap-1", "goal", 100);
    updated.summary = "rewritten".into();
    updated.updated_at = 300;
    assert!(
        store
            .update_brain_capability_if_version(
                &updated,
                &[],
                &vec_bytes(4, "emb", le(&[f32::NAN, 0.0, 0.0, 0.0]), 2),
                Some(&detail.content_version()),
            )
            .await
            .is_err(),
        "guarded update must reject a non-finite payload"
    );
    // …and so does the plain vector upsert, leaving the stored row intact.
    for (dim, emb) in [
        (0, Vec::new()),
        (4, le(&[1.0, 0.0, 0.0])),
        (4, le(&[f32::NAN, 0.0, 0.0, 0.0])),
    ] {
        assert!(
            store
                .upsert_brain_vector("cap-1", dim, "emb", &emb, 3)
                .await
                .is_err(),
            "upsert must reject dim {dim} with {} emb bytes",
            emb.len()
        );
    }
    assert_eq!(
        store
            .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
            .await
            .unwrap()
            .len(),
        1,
        "rejected upserts must leave the surviving vector untouched"
    );

    // Valid payloads still pass the same gates afterwards.
    store
        .upsert_brain_vector("cap-1", 4, "emb", &le(&[0.0, 1.0, 0.0, 0.0]), 4)
        .await
        .unwrap();
    store
        .update_brain_capability_if_version(
            &updated,
            &[eng("cap-1", "new", 1)],
            &vec_write(4, "emb", &[1.0, 0.0, 0.0, 0.0], 5),
            Some(&detail.content_version()),
        )
        .await
        .unwrap();
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].capability.id, "cap-1");
    assert!(
        hits[0].distance.abs() < 1e-6,
        "the valid replacement must be live"
    );
}

/// Rows that already sit in storage corrupted (metadata that does not
/// decode, blob disagreeing with its dim, non-finite values) are skipped
/// by search — fewer hits, never an error, never a torn blob scored as a
/// shorter valid vector.
#[tokio::test]
async fn search_skips_corrupt_vector_rows_instead_of_failing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = RdbStore::open(tmp.path()).await.unwrap();
    for (idx, id) in ["cap-ok", "cap-junk", "cap-mismatch", "cap-nan"]
        .iter()
        .enumerate()
    {
        store
            .create_brain_capability(&cap(id, "goal", 100 + idx as i64), &[])
            .await
            .unwrap();
    }
    store
        .upsert_brain_vector("cap-ok", 4, "emb", &le(&[1.0, 0.0, 0.0, 0.0]), 1)
        .await
        .unwrap();

    // A second handle over the same directory writes rows the public API
    // refuses: undecodable metadata, a blob shorter than its dim claims,
    // and exact-length non-finite values.
    let db = RdbDb::open(tmp.path()).unwrap();
    db.write(move |s| {
        let mut batch = rocksdb::WriteBatch::default();
        tx::put_raw(
            &mut batch,
            &brain::vector_key("cap-junk"),
            b"\xff\xfe torn json",
        );
        let mismatched = serde_json::to_vec(&serde_json::json!({
            "dim": 4,
            "model": "emb",
            "emb": le(&[1.0, 0.0, 0.0]),
            "updated_at": 1,
        }))?;
        tx::put_raw(&mut batch, &brain::vector_key("cap-mismatch"), &mismatched);
        let non_finite = serde_json::to_vec(&serde_json::json!({
            "dim": 4,
            "model": "emb",
            "emb": le(&[f32::NAN, 0.0, 0.0, 0.0]),
            "updated_at": 1,
        }))?;
        tx::put_raw(&mut batch, &brain::vector_key("cap-nan"), &non_finite);
        tx::commit(s, batch)
    })
    .await
    .unwrap();

    // The one valid row still answers; the three corrupt ones are skipped
    // without failing the scan.
    let hits = store
        .search_brain_vectors("emb", &le(&[1.0, 0.0, 0.0, 0.0]), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "only the valid vector may be scored");
    assert_eq!(hits[0].capability.id, "cap-ok");
    assert!(hits[0].distance.abs() < 1e-6);

    // A query whose dimension matches none of the stored (valid) vectors
    // scores nothing rather than truncating to the shorter side.
    assert!(
        store
            .search_brain_vectors("emb", &le(&[1.0, 0.0]), 10)
            .await
            .unwrap()
            .is_empty(),
        "dimension-mismatched queries must not be truncated into scores"
    );
}
