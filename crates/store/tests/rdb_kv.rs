//! KV infrastructure contracts for the rdb (RocksDB) store backend:
//! JSON row codec, lexicographic ordinal keys, global/per-parent sequence
//! allocation, unique-index conflict mapping, batch atomicity, cascade
//! range deletes and the embedding blob codec. These are the engine-level
//! invariants every domain module above relies on.

use opencoder_store::rdb_store::engine::RdbDb;
use opencoder_store::rdb_store::{index, kv, seq, tx, vector};

async fn mem() -> RdbDb {
    RdbDb::open(tempfile::tempdir().unwrap().path()).unwrap()
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Row {
    text: String,
    nested: Vec<(u64, String)>,
}

#[tokio::test]
async fn json_row_codec_roundtrips_unicode_and_nested_values() {
    let row = Row {
        text: "摘要 summary — ünïcødé ✓ 中文字符".into(),
        nested: vec![(7, "seventy".into()), (u64::MAX, String::new())],
    };
    let bytes = kv::encode(&row).unwrap();
    assert!(bytes.len() > 8);
    assert_eq!(kv::decode::<Row>(&bytes).unwrap(), row);
    // Torn/truncated JSON is an error, never a silent default.
    assert!(kv::decode::<Row>(&bytes[..bytes.len() - 1]).is_err());
    assert!(kv::decode::<Row>(b"not json").is_err());
}

#[tokio::test]
async fn padded_ordinals_sort_lexicographically_across_boundaries() {
    let mut ords = [1000, 999, 10, i64::MAX, 0, 123_456_789];
    let padded: Vec<String> = ords.iter().map(|n| kv::pad_ord(*n)).collect();
    // Every ordinal is zero-padded to the fixed width.
    assert!(padded.iter().all(|p| p.len() == kv::ORD_WIDTH));
    // Lexicographic byte order equals numeric order at every boundary.
    ords.sort();
    let mut sorted: Vec<String> = ords.iter().map(|n| kv::pad_ord(*n)).collect();
    sorted.sort();
    assert_eq!(
        sorted,
        ords.iter().map(|n| kv::pad_ord(*n)).collect::<Vec<_>>()
    );
    assert!(kv::pad_ord(999).as_bytes() < kv::pad_ord(1000).as_bytes());
    assert!(kv::pad_ord(i64::MAX - 1).as_bytes() < kv::pad_ord(i64::MAX).as_bytes());

    // The physical engine agrees: 999 sorts before 1000 under one prefix.
    let db = mem().await;
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        tx::put_raw(&mut batch, &kv::message_key("s", 1000), b"h");
        tx::put_raw(&mut batch, &kv::message_key("s", 999), b"l");
        tx::put_raw(&mut batch, &kv::message_key("s", 1001), b"t");
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    let keys = db
        .read(|s| {
            Ok(tx::scan_prefix(s, &kv::message_prefix("s"))?
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>())
        })
        .await
        .unwrap();
    assert_eq!(keys.len(), 3);
    assert_eq!(
        seq::parse_ord(&keys[0], &kv::message_prefix("s")),
        Some(999)
    );
    assert_eq!(
        seq::parse_ord(&keys[2], &kv::message_prefix("s")),
        Some(1001)
    );
    assert_eq!(
        seq::parse_ord(b"msg/s/not-an-ordinal", &kv::message_prefix("s/")),
        None
    );
}

#[tokio::test]
async fn global_seq_is_monotonic_and_tolerates_holes() {
    let db = mem().await;
    async fn alloc(db: &RdbDb, family: &str, commit: bool, key: &str) -> anyhow::Result<i64> {
        let (family, key) = (family.to_string(), key.to_string());
        db.write(move |s| {
            let mut batch = rocksdb::WriteBatch::default();
            let n = seq::next_global(s, &mut batch, &family)?;
            if commit {
                tx::put_raw(&mut batch, key.as_bytes(), b"");
                tx::commit(s, batch)?;
            }
            // A dropped batch leaves the counter untouched: the hole is
            // skipped, monotonicity is what matters.
            Ok(n)
        })
        .await
    }
    // First allocation ever starts at 1.
    assert_eq!(alloc(&db, "messages", true, "a").await.unwrap(), 1);
    // Committed increments persist.
    assert_eq!(alloc(&db, "messages", true, "b").await.unwrap(), 2);
    // An aborted batch stages nothing: the number is simply re-offered
    // (committed values stay strictly monotonic; gaps come from callers
    // discarding an allocated number, which is tolerated).
    assert_eq!(alloc(&db, "messages", false, "c").await.unwrap(), 3);
    assert_eq!(alloc(&db, "messages", true, "d").await.unwrap(), 3);
    // Families are independent counters.
    assert_eq!(alloc(&db, "session_events", true, "e").await.unwrap(), 1);
    assert_eq!(alloc(&db, "session_events", true, "f").await.unwrap(), 2);
    assert_eq!(alloc(&db, "messages", true, "g").await.unwrap(), 4);
}

#[tokio::test]
async fn per_parent_child_seqs_restart_at_one_and_follow_the_key_range() {
    let db = mem().await;
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        for n in [1i64, 2, 3] {
            tx::put_raw(&mut batch, &kv::event_key("s1", n), b"");
        }
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    assert_eq!(seq::max_child(db.handle(), &kv::event_prefix("s1")), 3);
    assert_eq!(seq::next_child(db.handle(), &kv::event_prefix("s1")), 4);
    // A sibling parent's range is untouched: cascade deletes never leak.
    assert_eq!(seq::max_child(db.handle(), &kv::event_prefix("s2")), 0);
    assert_eq!(seq::next_child(db.handle(), &kv::event_prefix("s2")), 1);
    // after_ord builds the exclusive scan-from key at the boundary itself:
    // iterating from it (excl_from=true) starts strictly past the ordinal.
    assert_eq!(
        seq::after_ord(&kv::event_prefix("s1"), 1),
        kv::event_key("s1", 1)
    );
}

#[tokio::test]
async fn unique_index_rejects_a_second_owner_with_the_sql_failure_text() {
    let db = mem().await;
    let index_key = b"idx/demo/col/value-1";
    let row_a = b"tbl/row-a";
    let row_b = b"tbl/row-b";
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        index::claim_unique(s, &mut batch, index_key, row_a, "tbl.col")?;
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    assert_eq!(
        index::owner(db.handle(), index_key).unwrap().as_deref(),
        Some(&row_a[..])
    );

    // Re-claiming for the SAME row is idempotent (retry semantics).
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        index::claim_unique(s, &mut batch, index_key, row_a, "tbl.col")?;
        tx::commit(s, batch)
    })
    .await
    .unwrap();

    // A different owner reproduces the SQL backend's conflict text.
    let err = db
        .write(|s| {
            let mut batch = rocksdb::WriteBatch::default();
            index::claim_unique(s, &mut batch, index_key, row_b, "tbl.col")?;
            tx::commit(s, batch)
        })
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: tbl.col"),
        "{err}"
    );
    // The rejected batch changed nothing: the first owner still holds it.
    assert_eq!(
        index::owner(db.handle(), index_key).unwrap().as_deref(),
        Some(&row_a[..])
    );

    // Removing the index entry in a batch frees the key for a new owner.
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        index::remove(&mut batch, index_key);
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    assert!(index::owner(db.handle(), index_key).unwrap().is_none());
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        index::claim_unique(s, &mut batch, index_key, row_b, "tbl.col")?;
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    assert_eq!(
        index::owner(db.handle(), index_key).unwrap().as_deref(),
        Some(&row_b[..])
    );
    // i64-valued membership indexes round-trip.
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        index::put_i64(&mut batch, b"idx/demo/n/9", 42)?;
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    assert_eq!(
        index::owner_i64(db.handle(), b"idx/demo/n/9").unwrap(),
        Some(42)
    );
    assert_eq!(index::owner_str(db.handle(), b"idx/missing").unwrap(), None);
}

#[tokio::test]
async fn a_failing_batch_write_leaves_nothing_persisted() {
    let db = mem().await;
    // Seed one committed row; it must survive every later aborted batch.
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        tx::put_raw(&mut batch, b"tbl/keep", b"kept");
        tx::commit(s, batch)
    })
    .await
    .unwrap();

    // Stage several puts and deletes, then fail mid-work BEFORE commit.
    let err = db
        .write::<(), _>(|s| {
            let mut batch = rocksdb::WriteBatch::default();
            tx::put_raw(&mut batch, b"tbl/a", b"1");
            tx::put_raw(&mut batch, b"tbl/b", b"2");
            batch.delete(b"tbl/keep");
            if true {
                return Err(anyhow::anyhow!("simulated builder failure"));
            }
            tx::commit(s, batch)
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("simulated builder failure"));

    let state = db
        .read(|s| {
            Ok([
                rdb::store::get(s, b"", b"tbl/keep").unwrap(),
                rdb::store::get(s, b"", b"tbl/a").unwrap(),
                rdb::store::get(s, b"", b"tbl/b").unwrap(),
            ]
            .map(|v| v.is_some()))
        })
        .await
        .unwrap();
    assert_eq!(
        state,
        [true, false, false],
        "rollback-by-construction: neither the staged puts nor the staged delete landed"
    );
}

#[tokio::test]
async fn delete_range_cascades_children_of_one_parent_only() {
    let db = mem().await;
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        for (sess, n) in [("s1", 1i64), ("s1", 2), ("s2", 1), ("s2", 2)] {
            tx::put_raw(&mut batch, &kv::message_key(sess, n), b"m");
        }
        tx::put_raw(&mut batch, &kv::event_key("s1", 1), b"e");
        tx::put_raw(&mut batch, &kv::session_key("s1"), b"root");
        tx::commit(s, batch)
    })
    .await
    .unwrap();

    tx::delete_range(db.handle(), &kv::message_prefix("s1")).unwrap();
    let count = |prefix: &[u8]| tx::scan_prefix(db.handle(), prefix).unwrap().len();
    assert_eq!(count(&kv::message_prefix("s1")), 0, "children cascaded");
    assert_eq!(
        count(&kv::message_prefix("s2")),
        2,
        "sibling parent untouched"
    );
    assert_eq!(
        count(&kv::event_prefix("s1")),
        1,
        "other families untouched"
    );
    assert_eq!(count(&kv::session_key("s1")), 1, "root row untouched");
}

#[tokio::test]
async fn descending_scan_walks_reverse_order_from_the_seek_key() {
    let db = mem().await;
    db.write(|s| {
        let mut batch = rocksdb::WriteBatch::default();
        for n in [5i64, 3, 1, 4, 2] {
            tx::put_raw(&mut batch, &kv::event_key("s", n), b"");
        }
        tx::commit(s, batch)
    })
    .await
    .unwrap();
    // Reverse pagination window: the two events strictly below seq 4.
    let mut seen = Vec::new();
    tx::scan_down(db.handle(), &kv::event_key("s", 4), true, |key, _| {
        seen.push(seq::parse_ord(key, &kv::event_prefix("s")).unwrap());
        seen.len() < 2 // stop callback: false ends the scan
    })
    .unwrap();
    assert_eq!(
        seen,
        vec![3, 2],
        "descending, exclusive of the seek key, early-stop honored"
    );
}

#[tokio::test]
async fn vector_blob_roundtrips_and_top_k_orders_by_cosine_similarity() {
    let dims = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.707_106_77, 0.707_106_77, 0.0],
    ];
    let blob = vector::encode_f32_le(&dims[0]);
    assert_eq!(blob.len(), 12);
    assert_eq!(vector::decode_f32_le(&blob), dims[0]);
    // Torn tail bytes drop the partial dimension instead of panicking…
    assert_eq!(vector::decode_f32_le(&blob[..blob.len() - 1]).len(), 2);
    // …but never count as the declared dimension: strict decode demands
    // exactly `dim` finite f32s, so a truncated or over-long blob is not
    // silently scored as a shorter valid vector.
    assert_eq!(
        vector::decode_f32_le_exact(&blob, 3),
        Some(dims[0].to_vec())
    );
    assert_eq!(
        vector::decode_f32_le_exact(&blob[..blob.len() - 1], 3),
        None
    );
    assert_eq!(vector::decode_f32_le_exact(&blob, 2), None);
    let non_finite = vector::encode_f32_le(&[f32::NAN, 0.0, 0.0]);
    assert_eq!(vector::decode_f32_le_exact(&non_finite, 3), None);

    let query = vec![0.9, 0.1, 0.0];
    let candidates = [
        ("c", dims[0].as_slice()),
        ("c", dims[1].as_slice()),
        ("c", dims[2].as_slice()),
    ];
    let hits = vector::top_k(candidates, &query, 2);
    assert_eq!(hits.len(), 2);
    // The aligned axis wins over the diagonal; cosine bounds hold.
    assert!(hits[0].1 > hits[1].1);
    assert!(hits[0].1 >= 0.0 && hits[0].1 <= 1.0);
    // Dimensions must match exactly — no silent truncation to min length.
    assert_eq!(vector::cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
    // Non-finite values have no similarity, on either side of the pair.
    assert_eq!(vector::cosine(&[f32::NAN, 0.0], &[1.0, 0.0]), None);
    assert_eq!(vector::cosine(&[1.0, 0.0], &[0.0, f32::INFINITY]), None);
    // top_k skips unscorable candidates instead of ranking them, and k = 0
    // ranks nothing (no panic on the empty selection window).
    let mixed = [
        ("ok", dims[0].as_slice()),
        ("short", [1.0, 0.0].as_slice()),
        ("nan", [f32::NAN, 0.0, 0.0].as_slice()),
    ];
    let only_valid = vector::top_k(mixed, &query, 3);
    assert_eq!(only_valid.len(), 1);
    assert_eq!(only_valid[0].0, "ok");
    assert!(vector::top_k(mixed, &query, 0).is_empty());
    // The zero vector has no direction: similarity 0.
    assert_eq!(vector::cosine(&[0.0, 0.0], &[1.0, 2.0]), Some(0.0));
    assert!((vector::cosine(&[1.0, 0.0], &[2.0, 0.0]).unwrap() - 1.0).abs() < 1e-6);
}
