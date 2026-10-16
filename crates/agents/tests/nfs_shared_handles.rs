use nfsserve::{
    nfs::{nfsstat3, nfsstring},
    vfs::NFSFileSystem,
};
use opencoder_agents::agents_fs;

#[tokio::test]
async fn acl_clone_resolves_backend_handles_without_scanning_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let fs = agents_fs(dir.path().to_path_buf());
    // The transport clone exists before the backend learns this long path.
    let acl = fs.clone();
    let relative = "fixtures/nested-resource-directory/versioned-inputs/transport/shared-handles/long-path/payload.txt";
    std::fs::create_dir_all(dir.path().join(relative).parent().unwrap()).unwrap();
    std::fs::write(dir.path().join(relative), "fixture contents").unwrap();
    let mut id = fs.root_dir();
    for part in relative.split('/') {
        id = fs
            .lookup(id, &nfsstring(part.as_bytes().to_vec()))
            .await
            .unwrap();
    }
    let handle = fs.id_to_fh(id);
    assert!(handle.data.len() <= 64);
    assert_eq!(acl.fh_to_id(&handle).unwrap(), id);
    assert_eq!(acl.read(id, 0, 1024).await.unwrap().0, b"fixture contents");
    // Removing the tree makes any fallback walk fail deterministically.
    // Handle resolution still works; file operations correctly report missing.
    std::fs::remove_dir_all(dir.path()).unwrap();
    assert_eq!(acl.fh_to_id(&handle).unwrap(), id);
    assert!(matches!(
        acl.getattr(id).await,
        Err(nfsstat3::NFS3ERR_NOENT)
    ));
    let mut truncated = handle;
    truncated.data.pop();
    assert!(matches!(
        acl.fh_to_id(&truncated),
        Err(nfsstat3::NFS3ERR_BADHANDLE)
    ));
}
