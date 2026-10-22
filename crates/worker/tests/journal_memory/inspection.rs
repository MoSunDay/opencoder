use super::*;
use crate::{journal::Journal, DirectoryLayout};
use opencoder_core::fleet::ExecutionKind;
use serde_json::json;
use std::path::PathBuf;

fn fixture() -> (tempfile::TempDir, DirectoryLayout, PathBuf, Record) {
    let directory = tempfile::tempdir().unwrap();
    let layout = DirectoryLayout::new(directory.path().into(), None).unwrap();
    let path = layout
        .record_path(ExecutionKind::Agent, "agent-detail")
        .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let record = serde_json::from_value(json!({
        "assignment": {
            "index": {"id":"agent-detail","kind":"agent","node_id":"node-test",
                      "status":"done","created_at":1},
            "request": {"id":"agent-detail","kind":"agent","input":{"prompt":"fixture"}},
            "runtime": {"frozen":"x".repeat(1024 * 1024)}
        },
        "annotations":{},"result":{"marker":"initial"},"error":null,"events":[]
    }))
    .unwrap();
    (directory, layout, path, record)
}

#[test]
fn loaded_small_details_do_not_read_or_cache_the_frozen_body() {
    let (_directory, layout, path, record) = fixture();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let journal = Journal::load(layout).unwrap();
    let (bytes, entries) = journal.inspection_cache.lock().unwrap().usage();
    assert_eq!(entries, 1);
    assert!(bytes < MAX_VIEW_BYTES);
    let detail = journal
        .ticket("agent-detail")
        .unwrap()
        .unwrap()
        .inspect_sync()
        .unwrap();
    assert_eq!(detail.result, record.result);
    assert_eq!(detail.request, record.assignment.request);
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    // Native readers still return the complete frozen payload, and Inspect
    // has not rewritten the original file into a partial representation.
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        journal
            .get_sync("agent-detail")
            .unwrap()
            .unwrap()
            .assignment
            .runtime,
        record.assignment.runtime
    );
}

#[test]
fn fresh_saves_keep_small_details_current_without_loading_frozen_bodies() {
    let (_directory, layout, _path, mut record) = fixture();
    let mut journal = Journal::load(layout).unwrap();
    for marker in ["accepted", "running", "completed"] {
        record.result = json!({"marker":marker});
        journal.save(record.clone()).unwrap();
        let detail = journal
            .ticket("agent-detail")
            .unwrap()
            .unwrap()
            .inspect_sync()
            .unwrap();
        assert_eq!(detail.result, record.result);
        assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
        assert_eq!(journal.inspection_cache.lock().unwrap().usage().1, 1);
    }
    assert_eq!(
        journal
            .get_sync("agent-detail")
            .unwrap()
            .unwrap()
            .assignment
            .runtime,
        record.assignment.runtime
    );
}

#[cfg(unix)]
#[test]
fn published_stamp_tracks_the_written_inode_and_rejects_content_changes() {
    use crate::journal::cache::Stamp;
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let temporary = directory.path().join("temporary");
    let target = directory.path().join("target");
    let mut file = std::fs::File::create(&temporary).unwrap();
    file.write_all(b"initial").unwrap();
    let before = file.metadata().unwrap();
    std::fs::rename(&temporary, &target).unwrap();
    let saved = Stamp::published(&before, &file.metadata().unwrap()).unwrap();
    assert!(saved == Stamp::of(&std::fs::metadata(&target).unwrap()));
    // Even a replacement before cache installation cannot lend its identity
    // to the view of the inode that we originally wrote.
    std::fs::write(&temporary, b"changed").unwrap();
    std::fs::rename(&temporary, &target).unwrap();
    let still_ours = Stamp::published(&before, &file.metadata().unwrap()).unwrap();
    assert!(still_ours != Stamp::of(&std::fs::metadata(&target).unwrap()));
    file.write_all(b"more").unwrap();
    assert!(Stamp::published(&before, &file.metadata().unwrap()).is_none());
}

#[test]
fn save_delete_and_external_replacement_invalidate_small_details() {
    let (_directory, layout, path, mut record) = fixture();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let mut journal = Journal::load(layout).unwrap();
    record.result = json!({"marker":"saved"});
    journal.save(record.clone()).unwrap();
    assert_eq!(journal.inspection_cache.lock().unwrap().usage().1, 1);
    assert_eq!(
        journal
            .ticket("agent-detail")
            .unwrap()
            .unwrap()
            .inspect_sync()
            .unwrap()
            .result,
        record.result
    );
    assert_eq!(journal.inspection_cache.lock().unwrap().usage().1, 1);
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    record.result = json!({"marker":"external-replacement"});
    let replacement = path.with_extension("replacement");
    std::fs::write(&replacement, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    assert_eq!(
        journal
            .ticket("agent-detail")
            .unwrap()
            .unwrap()
            .inspect_sync()
            .unwrap()
            .result,
        record.result
    );
    assert!(journal.forget("agent-detail").unwrap());
    assert_eq!(journal.inspection_cache.lock().unwrap().usage(), (0, 0));
    assert!(journal.ticket("agent-detail").unwrap().is_none());
}

#[test]
fn oversized_details_fall_back_without_changing_the_response() {
    let (_directory, layout, path, mut record) = fixture();
    record.result = json!({"body":"z".repeat(MAX_VIEW_BYTES * 4)});
    assert!(Inspection::encode(&record).is_none());
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let journal = Journal::load(layout).unwrap();
    assert_eq!(journal.inspection_cache.lock().unwrap().usage(), (0, 0));
    let detail = journal
        .ticket("agent-detail")
        .unwrap()
        .unwrap()
        .inspect_sync()
        .unwrap();
    assert_eq!(detail.result, record.result);
    assert_eq!(journal.inspection_cache.lock().unwrap().usage(), (0, 0));
    assert_eq!(journal.cache.lock().unwrap().usage().1, 1);
}

#[test]
fn cached_details_apply_the_current_durable_ack_overlay() {
    let (_directory, layout, path, mut record) = fixture();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut journal = Journal::load(layout).unwrap();
    // An independently committed ACK can be newer than the record file.
    record.annotations = json!({"brain_layered_ack": 7});
    journal.records.insert("agent-detail".into(), record);
    let detail = journal
        .ticket("agent-detail")
        .unwrap()
        .unwrap()
        .inspect_sync()
        .unwrap();
    assert_eq!(detail.annotations["brain_layered_ack"], 7);
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn detail_cache_enforces_both_payload_and_entry_limits() {
    let (_directory, _layout, path, record) = fixture();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let bytes = Inspection::encode(&record).unwrap();
    let stamp = crate::journal::cache::Stamp::of(&std::fs::metadata(path).unwrap());
    let mut cache = Cache::new(bytes.capacity() * 2, 3);
    for index in 0..100 {
        cache.insert(
            format!("agent-{index}"),
            1,
            stamp.clone(),
            std::sync::Arc::new(bytes.clone()),
        );
        let (retained, count) = cache.usage();
        assert!(retained <= bytes.capacity() * 2 && count <= 3);
    }
    assert!(cache.get("agent-0", 1, &stamp).is_none());
    assert!(cache.get("agent-99", 1, &stamp).is_some());
}

#[cfg(unix)]
#[test]
fn cached_details_still_reject_symlinks_and_replaced_identity() {
    let (_directory, layout, path, mut record) = fixture();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let journal = Journal::load(layout).unwrap();
    let ticket = journal.ticket("agent-detail").unwrap().unwrap();
    let moved = path.with_extension("moved");
    std::fs::rename(&path, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &path).unwrap();
    assert!(ticket.inspect_sync().is_err());
    std::fs::remove_file(&path).unwrap();
    record.assignment.index.node_id = "different-node".into();
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(ticket.inspect_sync().is_err());
}
