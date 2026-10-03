use opencoder_core::platform::fs;
use std::io::Write;

#[test]
fn private_files_replace_atomically_and_publish_once_under_concurrent_writers() {
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    fs::ensure_private_directory(&private).unwrap();
    assert!(fs::private_access(&private).unwrap());
    let target = private.join("snapshot.json");
    std::thread::scope(|scope| {
        for i in 0..8 {
            let private = &private;
            let target = &target;
            scope.spawn(move || {
                let source = private.join(format!("source-{i}"));
                let mut file = fs::create_private_file(&source).unwrap();
                writeln!(file, "{{\"winner\":{i}}}").unwrap();
                file.sync_all().unwrap();
                drop(file);
                if let Err(error) = fs::publish_new(&source, target) {
                    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                }
                let _ = std::fs::remove_file(source);
            });
        }
    });
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    assert!(value["winner"].as_u64().unwrap() < 8);
    assert!(fs::private_access(&target).unwrap());
    let source = private.join("replacement");
    let mut file = fs::create_private_file(&source).unwrap();
    file.write_all(b"replacement").unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::replace(&source, &target).unwrap();
    fs::sync_directory(&private).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"replacement");
    assert!(fs::private_access(&target).unwrap());
    assert_eq!(std::fs::read_dir(private).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_acl_rejects_public_access_and_junctions_and_supports_long_paths() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("private");
    fs::ensure_private_directory(&directory).unwrap();
    let file = directory.join("credential");
    drop(fs::create_private_file(&file).unwrap());
    assert!(fs::private_access(&file).unwrap());
    let status = std::process::Command::new("icacls.exe")
        .arg(&file)
        .args(["/grant", "*S-1-1-0:(R)"])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!fs::private_access(&file).unwrap());
    let link = root.path().join("junction");
    assert!(std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&directory)
        .status()
        .unwrap()
        .success());
    assert!(fs::is_link(&std::fs::symlink_metadata(&link).unwrap()));
    assert!(fs::ensure_private_directory(&link).is_err());
    let long = root
        .path()
        .join("中文目录".repeat(40))
        .join("第二目录".repeat(40))
        .join("private");
    fs::ensure_private_directory(&long).unwrap();
    let path = long.join("中文 file.txt");
    let mut file = fs::create_private_file(&path).unwrap();
    file.write_all(b"private").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(fs::private_access(&path).unwrap());
    assert_eq!(std::fs::read(path).unwrap(), b"private");
    for name in ["CON", "nul.json", "LPT1", "stream:name", "trim."] {
        assert!(!fs::valid_component(name), "{name}");
    }
    assert!(fs::valid_component("operator-01"));
}
