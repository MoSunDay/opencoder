const fn elf(payload: u8) -> [u8; 121] {
    let mut bytes = [0; 121];
    bytes[0] = 127;
    bytes[1] = b'E';
    bytes[2] = b'L';
    bytes[3] = b'F';
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[16] = 2;
    bytes[18] = 62;
    bytes[20] = 1;
    bytes[32] = 64;
    bytes[52] = 64;
    bytes[54] = 56;
    bytes[56] = 1;
    bytes[64] = 1;
    bytes[68] = 5;
    bytes[96] = 121;
    bytes[104] = 121;
    bytes[120] = payload;
    bytes
}

use opencoder_dag_binary::*;
use sha2::{Digest, Sha256};
use std::{io, path::Path};
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
use crate::meta::{read_pool_meta, read_version_meta};

/// A minimal valid module: 8-byte header + payload.
fn module(payload: &[u8]) -> Vec<u8> {
    let mut bytes = elf(0).to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

/// No `.tmp-` entries left in `dir` (torn-writer detector).
fn no_tmp_leftovers(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().contains(".tmp-"))
}

#[test]
fn save_creates_v1_with_correct_files_and_meta() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let bytes = module(b"(module)");
    assert_eq!(
        save_binary_version(root, "adder", "first cut", &bytes).unwrap(),
        1
    );
    assert_eq!(
        std::fs::read(version_dir(root, "adder", 1).join("binary.bin")).unwrap(),
        bytes
    );
    let vm = read_version_meta(root, "adder", 1).unwrap();
    assert_eq!(vm.version, 1);
    assert_eq!(vm.description, "first cut");
    assert_eq!(vm.sha256, sha256_hex(&bytes));
    assert_eq!(vm.size_bytes, bytes.len() as u64);
    let pm = read_pool_meta(root, "adder").unwrap();
    assert_eq!(pm.name, "adder");
    assert_eq!(pm.description, "first cut");
    assert_eq!(pm.current, 1);
    assert_eq!(pm.history, vec![1]);
    assert!(!pm.created_at.is_empty() && !pm.updated_at.is_empty());
    assert!(no_tmp_leftovers(&root.join("adder")));
}

#[test]
fn versions_are_monotonic_across_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    save_binary_version(root, "m", "d1", &module(b"one")).unwrap();
    assert_eq!(
        save_binary_version(root, "m", "d2", &module(b"two")).unwrap(),
        2
    );
    let pm = read_pool_meta(root, "m").unwrap();
    assert_eq!(pm.history, vec![1, 2]);
    assert_eq!(pm.description, "d2"); // pool description tracks latest
    rollback_binary(root, "m", 1).unwrap();
    let pm = read_pool_meta(root, "m").unwrap();
    assert_eq!(pm.current, 1);
    assert_eq!(pm.history, vec![1, 2]); // history intact
    assert!(version_dir(root, "m", 2).is_dir()); // dirs intact
    rollback_binary(root, "m", 1).unwrap(); // same-version no-op
    assert_eq!(read_pool_meta(root, "m").unwrap().current, 1);
    // Never reuse: after the rollback the next save is v3, not v2.
    assert_eq!(
        save_binary_version(root, "m", "d3", &module(b"three")).unwrap(),
        3
    );
    assert_eq!(read_pool_meta(root, "m").unwrap().history, vec![1, 2, 3]);
}

#[test]
fn save_rejects_bad_input_and_skips_unpublished_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert_eq!(
        save_binary_version(root, "../x", "d", &module(b"m"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    // Bad magic (\0max) → InvalidInput.
    assert_eq!(
        save_binary_version(root, "ok", "d", b"\0max\x01\0\0\0")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    std::fs::create_dir_all(root.join("ok/v1")).unwrap();
    assert_eq!(
        save_binary_version(root, "ok", "d", &module(b"m")).unwrap(),
        2
    );
    assert_eq!(read_pool_meta(root, "ok").unwrap().history, vec![2]);
    assert!(root.join("ok/v1").is_dir());
}

#[test]
fn failed_write_leaves_no_tmp_leftovers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join("adder");
    // Block the temp build: a directory named `binary.bin` inside the
    // exact temp path makes the atomic binary write fail (rename of a
    // file over a directory), deterministically and without relying
    // on filesystem permissions.
    std::fs::create_dir_all(
        dir.join(format!(".tmp-v1.{}", std::process::id()))
            .join("binary.bin"),
    )
    .unwrap();
    assert!(save_binary_version(root, "adder", "d", &module(b"m")).is_err());
    assert!(no_tmp_leftovers(&dir)); // temp dir removed entirely
    assert!(!dir.join("meta.json").exists()); // pool meta untouched
    assert!(!dir.join("v1").exists()); // nothing published
}

#[test]
fn rollback_rejects_unknown_pool_version_and_pruned_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    save_binary_version(root, "kit", "d", &module(b"m")).unwrap();
    assert_eq!(
        rollback_binary(root, "ghost", 1).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        rollback_binary(root, "kit", 7).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    // In history but the version dir was pruned out-of-band → rejected.
    save_binary_version(root, "kit", "d2", &module(b"m2")).unwrap();
    std::fs::remove_dir_all(version_dir(root, "kit", 2)).unwrap();
    assert_eq!(
        rollback_binary(root, "kit", 2).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        rollback_binary(root, "../x", 1).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn delete_is_idempotent_and_rejects_bad_names() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    save_binary_version(root, "gone", "d", &module(b"m")).unwrap();
    assert!(root.join("gone").is_dir());
    delete_binary(root, "gone").unwrap();
    delete_binary(root, "gone").unwrap(); // second delete is Ok
    assert!(!root.join("gone").exists());
    assert!(delete_binary(root, "never-there").is_ok());
    assert_eq!(
        delete_binary(root, "../x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn unpublished_version_is_never_reused_after_interrupted_publication() {
    let temp = tempfile::tempdir().unwrap();
    save_binary_version(temp.path(), "tool", "first", &elf(1)).unwrap();
    std::fs::create_dir(temp.path().join("tool/v2")).unwrap();
    assert_eq!(
        save_binary_version(temp.path(), "tool", "retry", &elf(2)).unwrap(),
        3
    );
    assert_eq!(
        read_pool_meta(temp.path(), "tool").unwrap().history,
        vec![1, 3]
    );
    assert!(rollback_binary(temp.path(), "tool", 2).is_err());
}

#[cfg(unix)]
#[test]
fn configured_pool_can_use_a_linked_data_parent_but_cannot_escape_through_pool_links() {
    let temporary = tempfile::tempdir().unwrap();
    let data = temporary.path().join("real-data");
    std::fs::create_dir(&data).unwrap();
    let alias = temporary.path().join("data");
    std::os::unix::fs::symlink(&data, &alias).unwrap();
    let root = alias.join("binaries");
    save_binary_version(&root, "tool", "native", &elf(1)).unwrap();
    assert!(data.join("binaries/tool/v1/binary.bin").is_file());
    let outside = temporary.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    assert_eq!(
        save_binary_version(&root, "escape", "native", &elf(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    std::os::unix::fs::symlink(&outside, root.join(".locks/lock-link")).unwrap();
    assert_eq!(
        save_binary_version(&root, "lock-link", "native", &elf(1))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn concurrent_creates_publish_exactly_one_version() {
    let temp = tempfile::tempdir().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let results = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let barrier = barrier.clone();
                let root = temp.path();
                scope.spawn(move || {
                    barrier.wait();
                    create_binary(root, "tool", "race", &elf(index))
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::AlreadyExists))
            .count(),
        7
    );
    assert_eq!(
        read_pool_meta(temp.path(), "tool").unwrap().history,
        vec![1]
    );
}
