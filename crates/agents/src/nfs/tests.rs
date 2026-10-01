//! Trait-level tests for the read-only agents-root VFS. The fixture is a
//! plain tempdir tree — no process-global agents-root override is touched,
//! so no `testutil::OVERRIDE_LOCK` is needed here.

use super::*;
use nfsserve::nfs::{ftype3, set_atime, set_gid3, set_mode3, set_mtime, set_size3, set_uid3};

#[path = "tests/handle_recovery.rs"]
mod handle_recovery;

#[test]
fn directory_errors_cannot_become_a_successful_partial_listing() {
    for (kind, expected) in [
        (
            std::io::ErrorKind::PermissionDenied,
            nfsstat3::NFS3ERR_ACCES,
        ),
        (std::io::ErrorKind::Other, nfsstat3::NFS3ERR_IO),
    ] {
        let entries = vec![Ok("first"), Err(std::io::Error::from(kind)), Ok("last")];
        assert_eq!(
            collect_names(entries.into_iter()).unwrap_err() as u32,
            expected as u32
        );
    }
}

/// `sattr3` with every field "don't change" — mutations must still be
/// rejected, proving ROFS does not depend on the requested attributes.
fn sattr_void() -> sattr3 {
    sattr3 {
        mode: set_mode3::Void,
        uid: set_uid3::Void,
        gid: set_gid3::Void,
        size: set_size3::Void,
        atime: set_atime::DONT_CHANGE,
        mtime: set_mtime::DONT_CHANGE,
    }
}

/// Fixture: `<root>/meta.json` + `prompts/p/v1/soul.md` + a symlink.
fn fixture() -> (tempfile::TempDir, ReadOnlyAgentsFs) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.json"), br#"{"name":"p"}"#).unwrap();
    let v1 = dir.path().join("prompts/p/v1");
    std::fs::create_dir_all(&v1).unwrap();
    std::fs::write(v1.join("soul.md"), b"be terse").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("meta.json", dir.path().join("active")).unwrap();
    let fs = agents_fs(dir.path().to_path_buf());
    (dir, fs)
}

fn name(s: &str) -> filename3 {
    nfsstring(s.as_bytes().to_vec())
}

#[tokio::test]
async fn walk_lookup_getattr_read() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    let prompts = fs.lookup(root, &name("prompts")).await.unwrap();
    let p = fs.lookup(prompts, &name("p")).await.unwrap();
    let v1 = fs.lookup(p, &name("v1")).await.unwrap();
    let soul = fs.lookup(v1, &name("soul.md")).await.unwrap();

    let attr = fs.getattr(soul).await.unwrap();
    assert!(matches!(attr.ftype, ftype3::NF3REG));
    assert_eq!(attr.size, b"be terse".len() as u64);

    let (bytes, eof) = fs.read(soul, 0, 4096).await.unwrap();
    assert_eq!(bytes, b"be terse");
    assert!(eof);
    let (bytes, eof) = fs.read(soul, 3, 2).await.unwrap();
    assert_eq!(bytes, b"te");
    assert!(!eof);
    let (bytes, eof) = fs.read(soul, 100, 4).await.unwrap();
    assert_eq!(bytes, b"");
    assert!(eof);

    let attr = fs.getattr(root).await.unwrap();
    assert!(matches!(attr.ftype, ftype3::NF3DIR));
}

#[tokio::test]
async fn readdir_lists_and_paginates() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    let res = fs.readdir(root, 0, 100).await.unwrap();
    let listed: Vec<String> = res
        .entries
        .iter()
        .map(|e| String::from_utf8(e.name.0.clone()).unwrap())
        .collect();
    assert!(listed.contains(&"meta.json".to_string()));
    assert!(listed.contains(&"prompts".to_string()));
    assert!(res.end);

    // One entry per page: every page but the last flags more data, and
    // walking cookies covers the whole directory exactly once.
    let mut seen: Vec<String> = Vec::new();
    let mut cookie = 0;
    loop {
        let page = fs.readdir(root, cookie, 1).await.unwrap();
        assert_eq!(page.entries.len(), 1);
        seen.push(String::from_utf8(page.entries[0].name.0.clone()).unwrap());
        cookie = page.entries[0].fileid;
        if page.end {
            break;
        }
    }
    assert_eq!(seen.len(), listed.len());
    assert_eq!(seen, listed);
}

#[tokio::test]
async fn lookup_missing_and_traversal_rejected() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    assert!(matches!(
        fs.lookup(root, &name("nope")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name("..")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name(".")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name("a/b")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name("/etc")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name("")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &name("\0x")).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.lookup(root, &nfsstring(vec![0xff, 0xfe]))
            .await
            .unwrap_err(),
        nfsstat3::NFS3ERR_NOENT
    ));
    assert!(matches!(
        fs.getattr(987654321).await.unwrap_err(),
        nfsstat3::NFS3ERR_STALE
    ));
}

#[tokio::test]
async fn every_mutating_op_is_rofs() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    let soul = {
        let prompts = fs.lookup(root, &name("prompts")).await.unwrap();
        let p = fs.lookup(prompts, &name("p")).await.unwrap();
        let v1 = fs.lookup(p, &name("v1")).await.unwrap();
        fs.lookup(v1, &name("soul.md")).await.unwrap()
    };
    assert!(matches!(
        fs.setattr(soul, sattr_void()).await.unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.write(soul, 0, b"x").await.unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.create(root, &name("n.txt"), sattr_void())
            .await
            .unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.create_exclusive(root, &name("n.txt")).await.unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.mkdir(root, &name("n")).await.unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.remove(root, &name("meta.json")).await.unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.rename(root, &name("meta.json"), root, &name("x"))
            .await
            .unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
    assert!(matches!(
        fs.symlink(root, &name("l"), &name("t"), &sattr_void())
            .await
            .unwrap_err(),
        nfsstat3::NFS3ERR_ROFS
    ));
}

#[tokio::test]
#[cfg(unix)]
async fn symlink_surfaced_via_readlink() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    let link = fs.lookup(root, &name("active")).await.unwrap();
    assert!(matches!(
        fs.getattr(link).await.unwrap().ftype,
        ftype3::NF3LNK
    ));
    assert_eq!(fs.readlink(link).await.unwrap().0, b"meta.json".to_vec());
    let meta = fs.lookup(root, &name("meta.json")).await.unwrap();
    assert!(matches!(
        fs.readlink(meta).await.unwrap_err(),
        nfsstat3::NFS3ERR_INVAL
    ));
}

#[tokio::test]
async fn wrong_type_ops_rejected() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    let meta = fs.lookup(root, &name("meta.json")).await.unwrap();
    // READDIR on a regular file, READ on a directory.
    assert!(matches!(
        fs.readdir(meta, 0, 10).await.unwrap_err(),
        nfsstat3::NFS3ERR_NOTDIR
    ));
    assert!(matches!(
        fs.read(root, 0, 16).await.unwrap_err(),
        nfsstat3::NFS3ERR_ISDIR
    ));
}
