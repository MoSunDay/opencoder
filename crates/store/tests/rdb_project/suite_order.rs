//! Initiative board reordering: CAS on the exact current id list, rejection
//! of stale/omitted/duplicate/foreign lists, and persistence across reopen
//! (ported from the SQL order suite; the SQL trigger fault injection stays
//! SQL-only — rdb stages every row write in one atomic batch).
use super::*;
use opencoder_store::project::summary::SummaryScope;
use opencoder_store::project::InitiativeOrderConflict;

async fn seed(s: &dyn ProjectStore) {
    for id in ["p", "other"] {
        s.create_goal(&goal(id, 0, 1)).await.unwrap();
    }
    for (id, goal) in [("a", "p"), ("b", "p"), ("c", "p"), ("outside", "other")] {
        let mut rec = initiative(id, Some(goal), 0, 1);
        rec.summary.current_summary = "saved".into();
        rec.summary.summary_updated_at = Some(1);
        s.create_initiative(&rec).await.unwrap();
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|id| id.to_string()).collect()
}

async fn order(s: &dyn ProjectStore) -> Vec<String> {
    s.list_initiatives(Some("p"))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[tokio::test]
async fn initiative_order_persists_and_rejects_stale_incomplete_or_foreign_lists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order-db");
    let s = RdbStore::open(&path).await.unwrap();
    seed(&s).await;
    let original = ids(&["a", "b", "c"]);
    let next = ids(&["c", "a", "b"]);
    assert_eq!(order(&s).await, original);
    assert!(s
        .reorder_initiatives("p", &original, &next, 20)
        .await
        .unwrap());
    assert_eq!(order(&s).await, next);
    for (expected, invalid) in [
        (original.clone(), original),                // stale order
        (next.clone(), ids(&["a", "b"])),            // omitted initiative
        (next.clone(), ids(&["a", "a", "b"])),       // duplicate
        (next.clone(), ids(&["a", "b", "outside"])), // foreign initiative
    ] {
        let error = s
            .reorder_initiatives("p", &expected, &invalid, 20)
            .await
            .unwrap_err();
        assert!(error.is::<InitiativeOrderConflict>());
        assert_eq!(order(&s).await, next);
    }
    // Unknown goal: false, not an error.
    assert!(!s
        .reorder_initiatives("missing", &[], &[], 30)
        .await
        .unwrap());
    // Other goals are untouched by the reorder.
    let outside = s.list_initiatives(Some("other")).await.unwrap().remove(0);
    assert_eq!(outside.sort, 0);
    assert_eq!(outside.updated_at, 1);
    for row in s.list_initiatives(Some("p")).await.unwrap() {
        assert_eq!(row.summary.current_summary, "saved");
        assert_eq!(row.summary.summary_updated_at, Some(1));
        assert_eq!(
            s.summary_versions(SummaryScope::Initiative, &row.id)
                .await
                .unwrap()
                .unwrap()
                .len(),
            1
        );
    }
    drop(s);
    let s = RdbStore::open(&path).await.unwrap();
    assert_eq!(order(&s).await, next);
}

/// rdb 替代 SQL 触发器故障注入：两个句柄打开同一目录，第二个句柄在
/// 幕后改写一行（行内 id 与键不一致），重排的前置 CAS 校验把这种
/// 陈旧列表拦下，所有行保持原序、时间戳不动。
#[tokio::test]
async fn behind_the_store_row_tampering_is_rejected_by_the_cas() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tamper-db");
    let s = RdbStore::open(&path).await.unwrap();
    seed(&s).await;
    let mut row = s
        .list_initiatives(Some("p"))
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == "b")
        .unwrap();
    row.id = "renamed".into();
    drop(s);
    // RocksDB locks the directory per handle: tamper with a raw handle and
    // reopen the store afterwards.
    let db = opencoder_store::rdb_store::RdbDb::open(&path).unwrap();
    db.write(move |store| {
        let mut batch = rocksdb::WriteBatch::default();
        opencoder_store::rdb_store::tx::put(&mut batch, b"pin/b", &row)?;
        opencoder_store::rdb_store::tx::commit(store, batch)
    })
    .await
    .unwrap();
    drop(db);
    let s = RdbStore::open(&path).await.unwrap();
    // The board now reads a different id set, so a reorder built on the
    // stale picture conflicts instead of clobbering rows.
    let error = s
        .reorder_initiatives("p", &ids(&["a", "b", "c"]), &ids(&["c", "b", "a"]), 20)
        .await
        .unwrap_err();
    assert!(error.is::<InitiativeOrderConflict>());
    assert!(s
        .list_initiatives(Some("p"))
        .await
        .unwrap()
        .iter()
        .all(|row| row.updated_at == 1));
}
