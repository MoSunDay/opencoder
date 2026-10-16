use super::*;

#[test]
fn unknown_opaque_handle_is_stale_when_the_export_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let fs = agents_fs(dir.path().to_path_buf());
    let handle = nfs_fh3 {
        data: handles::encode(&PathBuf::from(
            "a-resource-path-longer-than-sixty-bytes-that-the-export-has-not-registered/file",
        )),
    };
    std::fs::remove_dir(dir.path()).unwrap();
    assert!(matches!(fs.fh_to_id(&handle), Err(nfsstat3::NFS3ERR_STALE)));
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn unknown_opaque_handle_does_not_open_or_read_the_export_directory() {
    use std::{
        ffi::CString,
        io::Read,
        os::fd::{AsRawFd, FromRawFd},
    };

    let (dir, original) = fixture();
    let relative = "a-complete-resource-package-with-a-name-longer-than-sixty-bytes";
    std::fs::write(dir.path().join(relative), "body").unwrap();
    let id = original
        .lookup(original.root_dir(), &name(relative))
        .await
        .unwrap();
    let mut handle = original.id_to_fh(id);
    let fresh = agents_fs(dir.path().to_path_buf());

    // Observe actual directory access, including scans that happen to return STALE.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    assert!(fd >= 0, "{}", std::io::Error::last_os_error());
    let mut events = unsafe { std::fs::File::from_raw_fd(fd) };
    let path = CString::new(dir.path().as_os_str().as_encoded_bytes()).unwrap();
    let watch = unsafe {
        libc::inotify_add_watch(
            events.as_raw_fd(),
            path.as_ptr(),
            libc::IN_OPEN | libc::IN_ACCESS,
        )
    };
    assert!(watch >= 0, "{}", std::io::Error::last_os_error());
    for fs in [&fresh, &original] {
        let result = fs.fh_to_id(&handle);
        let mut buffer = [0u8; 1024];
        assert_eq!(
            events.read(&mut buffer).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(matches!(result, Err(nfsstat3::NFS3ERR_STALE)));
        // A valid-sized handle with an unknown digest must also avoid directory I/O.
        *handle.data.last_mut().unwrap() ^= 1;
    }
}

#[tokio::test]
async fn long_directory_pagination_recovers_after_restart_with_fresh_lookup() {
    let (dir, fs) = fixture();
    let relative = "skills/a-complete-resource-package-with-a-long-name/v1/references";
    std::fs::create_dir_all(dir.path().join(relative)).unwrap();
    for i in 0..37 {
        std::fs::write(
            dir.path()
                .join(relative)
                .join(format!("contract-{i:02}.md")),
            "body",
        )
        .unwrap();
    }
    let mut id = fs.root_dir();
    for part in relative.split('/') {
        id = fs.lookup(id, &name(part)).await.unwrap();
    }
    let old_handle = fs.id_to_fh(id);
    let restarted = agents_fs(dir.path().to_path_buf());
    assert!(matches!(
        restarted.fh_to_id(&old_handle),
        Err(nfsstat3::NFS3ERR_STALE)
    ));
    let mut fresh_id = restarted.root_dir();
    for part in relative.split('/') {
        fresh_id = restarted.lookup(fresh_id, &name(part)).await.unwrap();
    }
    assert_eq!(fresh_id, id);
    assert_eq!(restarted.id_to_fh(fresh_id).data, old_handle.data);
    let id = restarted.fh_to_id(&old_handle).unwrap();
    let mut cookie = 0;
    let mut names = Vec::new();
    loop {
        let page = restarted.readdir(id, cookie, 4).await.unwrap();
        assert!(!page.entries.is_empty());
        for entry in page.entries {
            cookie = entry.fileid;
            assert_eq!(restarted.read(cookie, 0, 1024).await.unwrap().0, b"body");
            names.push(entry.name.0);
        }
        if page.end {
            break;
        }
        assert!(names.len() < 37);
    }
    assert_eq!(names.len(), 37);
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 37);
    let old = nfs_fh3 {
        data: b"prompts/p/v1/soul.md".to_vec(),
    };
    let id = restarted.fh_to_id(&old).unwrap();
    assert_eq!(restarted.id_to_fh(id).data, old.data);
    assert_eq!(restarted.read(id, 0, 1024).await.unwrap().0, b"be terse");
}

#[tokio::test]
async fn handle_roundtrip_and_rejection() {
    let (_dir, fs) = fixture();
    let root = fs.root_dir();
    // Root handle is the lone "/" (a zero-length handle is EBADHANDLE for
    // the Linux kernel) and roundtrips through the id.
    let fh = fs.id_to_fh(root);
    assert_eq!(fh.data, b"/");
    assert_eq!(fs.fh_to_id(&fh).unwrap(), root);

    let prompts = fs.lookup(root, &name("prompts")).await.unwrap();
    let fh = fs.id_to_fh(prompts);
    assert_eq!(fh.data, b"prompts");
    assert_eq!(fs.fh_to_id(&fh).unwrap(), prompts);

    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: b"/etc/passwd".to_vec()
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: b"a/../b".to_vec()
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: b"./a".to_vec()
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: b"a//b".to_vec()
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: vec![0xff, 0xfe]
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 { data: vec![0] }).unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    assert!(matches!(
        fs.fh_to_id(&nfs_fh3 {
            data: vec![b'x'; 61]
        })
        .unwrap_err(),
        nfsstat3::NFS3ERR_BADHANDLE
    ));
    // An id the registry never handed out must not alias the root.
    assert_eq!(fs.id_to_fh(42).data, vec![0]);
}

#[tokio::test]
async fn complete_skill_package_survives_restart_with_fresh_lookup() {
    let (dir, fs) = fixture();
    let relative = "skills/sample-workflow/v2/sample-analysis/references/service-contract.md";
    std::fs::create_dir_all(dir.path().join(relative).parent().unwrap()).unwrap();
    std::fs::write(dir.path().join(relative), "real workflow contract").unwrap();
    assert!(relative.len() > MAX_FH_PATH);
    let mut parent = fs.root_dir();
    for component in relative.split('/') {
        let listed = fs.readdir(parent, 0, 100).await.unwrap();
        assert!(listed
            .entries
            .iter()
            .any(|e| e.name.0 == component.as_bytes()));
        parent = fs.lookup(parent, &name(component)).await.unwrap();
        let handle = fs.id_to_fh(parent);
        assert!(handle.data.len() <= 64);
        assert_eq!(fs.fh_to_id(&handle).unwrap(), parent);
    }
    let handle = fs.id_to_fh(parent);
    let restarted = agents_fs(dir.path().to_path_buf());
    assert!(matches!(
        restarted.fh_to_id(&handle),
        Err(nfsstat3::NFS3ERR_STALE)
    ));
    let mut fresh_id = restarted.root_dir();
    for component in relative.split('/') {
        fresh_id = restarted.lookup(fresh_id, &name(component)).await.unwrap();
    }
    assert_eq!(fresh_id, parent);
    let recovered = restarted.fh_to_id(&handle).unwrap();
    assert_eq!(recovered, parent);
    assert_eq!(
        restarted.read(recovered, 0, 1024).await.unwrap().0,
        b"real workflow contract"
    );
    std::fs::remove_file(dir.path().join(relative)).unwrap();
    let restarted = agents_fs(dir.path().to_path_buf());
    assert!(matches!(
        restarted.fh_to_id(&handle),
        Err(nfsstat3::NFS3ERR_STALE)
    ));
    let mut truncated = handle.clone();
    truncated.data.pop();
    assert!(matches!(
        restarted.fh_to_id(&truncated),
        Err(nfsstat3::NFS3ERR_BADHANDLE)
    ));
}

#[tokio::test]
#[cfg(unix)]
async fn opaque_handle_cannot_follow_a_link_outside_the_export() {
    let (dir, fs) = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "outside").unwrap();
    let link = "a-long-skill-name-that-makes-the-relative-resource-path-exceed-sixty-bytes";
    std::os::unix::fs::symlink(outside.path(), dir.path().join(link)).unwrap();
    let handle = nfs_fh3 {
        data: handles::encode(&PathBuf::from(link).join("secret.md")),
    };
    assert!(matches!(fs.fh_to_id(&handle), Err(nfsstat3::NFS3ERR_STALE)));
    let link_id = fs.lookup(fs.root_dir(), &name(link)).await.unwrap();
    assert!(matches!(
        fs.lookup(link_id, &name("secret.md")).await,
        Err(nfsstat3::NFS3ERR_NOTDIR)
    ));
    std::os::unix::fs::symlink(outside.path(), dir.path().join("short")).unwrap();
    let forged = fs
        .fh_to_id(&nfs_fh3 {
            data: b"short/secret.md".to_vec(),
        })
        .unwrap();
    assert!(matches!(
        fs.read(forged, 0, 1024).await,
        Err(nfsstat3::NFS3ERR_ACCES)
    ));
}
