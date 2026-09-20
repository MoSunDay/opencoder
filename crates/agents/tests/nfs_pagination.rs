//! Exercise RPC pagination, including the plain READDIR selected by Linux for
//! larger directories. Testing the VFS alone misses a dropped wire cookie.
use opencoder_agents::{spawn_nfs_server, NfsServerHandle, NfsServerOpts};
use std::collections::BTreeSet;
use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

fn start(root: &Path, port: u16) -> NfsServerHandle {
    spawn_nfs_server(&NfsServerOpts {
        export_root: root.into(),
        host: "127.0.0.1".into(),
        port,
        read_only: true,
    })
    .unwrap()
}

fn word(input: &mut Cursor<Vec<u8>>) -> u32 {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes).unwrap();
    u32::from_be_bytes(bytes)
}

fn wide(input: &mut Cursor<Vec<u8>>) -> u64 {
    (u64::from(word(input)) << 32) | u64::from(word(input))
}

fn opaque(input: &mut Cursor<Vec<u8>>) -> Vec<u8> {
    let size = word(input) as usize;
    assert!(size <= 4096);
    let mut bytes = vec![0; size.div_ceil(4) * 4];
    input.read_exact(&mut bytes).unwrap();
    bytes.truncate(size);
    bytes
}

fn attributes(input: &mut Cursor<Vec<u8>>) {
    if word(input) != 0 {
        input.read_exact(&mut [0; 84]).unwrap();
    }
}

fn page(port: u16, procedure: u32, cookie: u64, xid: u32) -> (Vec<(String, u64)>, bool) {
    let mut request: Vec<u8> = [xid, 0, 2, 100003, 3, procedure, 0, 0, 0, 0]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    // Stable root handle: an XDR opaque string containing "/".
    request.extend_from_slice(&[0, 0, 0, 1, b'/', 0, 0, 0]);
    request.extend_from_slice(&cookie.to_be_bytes());
    request.extend_from_slice(&[0; 8]);
    request.extend_from_slice(&512_u32.to_be_bytes());
    if procedure == 17 {
        request.extend_from_slice(&4096_u32.to_be_bytes());
    }
    let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .write_all(&(0x80000000 | request.len() as u32).to_be_bytes())
        .unwrap();
    socket.write_all(&request).unwrap();
    let mut response = Vec::new();
    loop {
        let mut header = [0; 4];
        socket.read_exact(&mut header).unwrap();
        let fragment = u32::from_be_bytes(header);
        let size = (fragment & 0x7fffffff) as usize;
        assert!(size < 16384);
        let offset = response.len();
        response.resize(offset + size, 0);
        socket.read_exact(&mut response[offset..]).unwrap();
        if fragment & 0x80000000 != 0 {
            break;
        }
    }
    let mut response = Cursor::new(response);
    for expected in [xid, 1, 0, 0, 0, 0, 0] {
        assert_eq!(word(&mut response), expected, "RPC / NFS status");
    }
    attributes(&mut response);
    wide(&mut response); // cookie verifier
    let mut entries = Vec::new();
    while word(&mut response) != 0 {
        wide(&mut response); // file ID
        let name = String::from_utf8(opaque(&mut response)).unwrap();
        let next = wide(&mut response);
        if procedure == 17 {
            attributes(&mut response);
            if word(&mut response) != 0 {
                opaque(&mut response);
            }
        }
        entries.push((name, next));
    }
    let end = word(&mut response) != 0;
    assert_eq!(response.position() as usize, response.get_ref().len());
    (entries, end)
}

fn listing(procedure: u32, replace_exporter: bool) {
    let root = tempfile::tempdir().unwrap();
    let expected: BTreeSet<_> = (0..300)
        .map(|index| format!("resource-{index:04}-long-directory-name"))
        .collect();
    for name in &expected {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    let mut server = start(root.path(), 0);
    let port = server.local_addr().unwrap().port();
    let mut seen = BTreeSet::new();
    let mut cookie = 0;
    let mut reached_end = false;
    for index in 0..100 {
        let (entries, end) = page(port, procedure, cookie, index + 1);
        assert!(!entries.is_empty() || end, "empty page must terminate");
        for (name, next) in entries {
            assert!(seen.insert(name.clone()), "repeated directory entry {name}");
            assert_ne!(next, cookie, "pagination must advance");
            cookie = next;
        }
        if end {
            reached_end = true;
            break;
        }
        if replace_exporter && index == 0 {
            server.shutdown();
            server = start(root.path(), port);
        }
    }
    server.shutdown();
    assert!(reached_end, "directory pagination did not terminate");
    assert_eq!(seen, expected, "no missing or duplicated resources");
}

#[test]
fn plain_readdir_advances_every_page() {
    listing(16, false);
}

#[test]
fn readdirplus_advances_every_page() {
    listing(17, false);
}

#[test]
fn plain_readdir_cookie_survives_exporter_replacement() {
    listing(16, true);
}

#[test]
#[ignore = "manual: needs mount privileges + nfs client"]
fn kernel_plain_readdir_continues_across_exporter_replacement() {
    use std::process::Command;
    struct Mounted {
        mount: tempfile::TempDir,
        server: Option<NfsServerHandle>,
    }
    impl Drop for Mounted {
        fn drop(&mut self) {
            let result = Command::new("timeout")
                .args(["10", "umount"])
                .arg(self.mount.path())
                .status()
                .unwrap();
            assert!(result.success(), "private test mount must be released");
            self.server.take().unwrap().shutdown();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let expected: BTreeSet<_> = (0..300)
        .map(|index| format!("resource-{index:04}-long-directory-name"))
        .collect();
    for name in &expected {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    let server = start(root.path(), 0);
    let port = server.local_addr().unwrap().port();
    let mount = tempfile::tempdir().unwrap();
    let status = Command::new("timeout")
        .args(["10", "mount", "-t", "nfs", "-o"])
        .arg(format!("ro,vers=3,tcp,port={port},mountport={port},nolock,soft,retrans=1,timeo=50,nordirplus,actimeo=0,lookupcache=none"))
        .arg("127.0.0.1:/")
        .arg(mount.path())
        .status()
        .unwrap();
    assert!(status.success(), "private NFS mount must succeed");
    let mut mounted = Mounted {
        mount,
        server: Some(server),
    };
    let mut entries = std::fs::read_dir(mounted.mount.path()).unwrap();
    let mut seen = BTreeSet::new();
    for index in 0..300 {
        let entry = entries.next().expect("missing entry").unwrap();
        assert!(seen.insert(entry.file_name().to_string_lossy().into_owned()));
        if index == 10 {
            mounted.server.take().unwrap().shutdown();
            mounted.server = Some(start(root.path(), port));
        }
    }
    assert!(entries.next().is_none(), "must reach EOF");
    assert_eq!(seen, expected);
    drop(entries);
    drop(mounted);
}
