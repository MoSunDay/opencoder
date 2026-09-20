//! Exercise the actual RPC handlers, including the plain READDIR path selected
//! by Linux for larger directories. VFS-only tests cannot detect lost cookies.
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use opencoder_agents::{spawn_nfs_server, NfsServerOpts};

fn u32_xdr(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

fn opaque(out: &mut Vec<u8>, value: &[u8]) {
    u32_xdr(out, value.len() as u32);
    out.extend(value);
    out.resize(out.len().next_multiple_of(4), 0);
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, size: usize) -> &'a [u8] {
        let bytes = &self.bytes[self.offset..self.offset + size];
        self.offset += size;
        bytes
    }

    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().unwrap())
    }

    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.take(8).try_into().unwrap())
    }

    fn opaque(&mut self) -> &'a [u8] {
        let size = self.u32() as usize;
        let value = self.take(size);
        self.take((4 - size % 4) % 4);
        value
    }

    fn attributes(&mut self) {
        if self.u32() != 0 {
            self.take(84);
        }
    }
}

fn rpc(stream: &mut TcpStream, xid: u32, procedure: u32, cookie: u64, verifier: &[u8]) -> Vec<u8> {
    let mut request = Vec::new();
    for number in [xid, 0, 2, 100003, 3, procedure, 0, 0, 0, 0] {
        u32_xdr(&mut request, number);
    }
    opaque(&mut request, b"/");
    request.extend(cookie.to_be_bytes());
    request.extend(verifier);
    u32_xdr(&mut request, 2048);
    if procedure == 17 {
        u32_xdr(&mut request, 8192);
    }
    stream
        .write_all(&(0x8000_0000 | request.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&request).unwrap();
    let mut response = Vec::new();
    loop {
        let mut marker = [0; 4];
        stream.read_exact(&mut marker).unwrap();
        let marker = u32::from_be_bytes(marker);
        let count = (marker & 0x7fff_ffff) as usize;
        assert!(count <= 1 << 20);
        let offset = response.len();
        response.resize(offset + count, 0);
        stream.read_exact(&mut response[offset..]).unwrap();
        if marker & 0x8000_0000 != 0 {
            return response;
        }
    }
}

fn enumerate(procedure: u32) {
    let root = tempfile::tempdir().unwrap();
    let expected: BTreeSet<_> = (0..136)
        .map(|index| format!("resource-{index:04}-a-name-that-requires-multiple-directory-pages"))
        .collect();
    for name in &expected {
        std::fs::write(root.path().join(name), b"frozen resource").unwrap();
    }
    let server = spawn_nfs_server(&NfsServerOpts {
        export_root: root.path().to_owned(),
        host: "127.0.0.1".into(),
        port: 0,
        read_only: true,
    })
    .unwrap();
    let mut stream = TcpStream::connect(server.local_addr().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut cookie = 0;
    let mut verifier = vec![0; 8];
    let mut observed = BTreeSet::new();
    let mut eof = false;
    let mut pages = 0;
    for xid in 1..=32 {
        let bytes = rpc(&mut stream, xid, procedure, cookie, &verifier);
        let mut decoder = Decoder {
            bytes: &bytes,
            offset: 0,
        };
        assert_eq!(decoder.u32(), xid);
        assert_eq!(decoder.u32(), 1); // REPLY
        assert_eq!(decoder.u32(), 0); // MSG_ACCEPTED
        decoder.u32(); // verifier flavor
        decoder.opaque();
        assert_eq!(decoder.u32(), 0); // RPC_SUCCESS
        assert_eq!(decoder.u32(), 0); // NFS3_OK
        decoder.attributes();
        verifier = decoder.take(8).to_vec();
        let previous = cookie;
        while decoder.u32() != 0 {
            decoder.u64(); // fileid
            let name = String::from_utf8(decoder.opaque().to_vec()).unwrap();
            cookie = decoder.u64();
            if procedure == 17 {
                decoder.attributes();
                if decoder.u32() != 0 {
                    decoder.opaque();
                }
            }
            assert!(
                observed.insert(name),
                "server repeated an entry after a pagination cookie"
            );
        }
        pages += 1;
        eof = decoder.u32() != 0;
        assert_eq!(decoder.offset, bytes.len());
        if eof {
            break;
        }
        assert_ne!(cookie, previous, "pagination made no progress");
    }
    server.shutdown();
    assert!(eof, "directory did not reach EOF");
    assert!(pages > 1, "fixture must exercise multiple wire pages");
    assert_eq!(observed, expected);
}

#[test]
fn plain_readdir_advances_cookies_without_duplicates_or_omissions() {
    enumerate(16);
}

#[test]
fn readdirplus_still_advances_cookies_without_duplicates_or_omissions() {
    enumerate(17);
}
