#![cfg(target_os = "linux")]
use opencoder_agents::{spawn_nfs_server, NfsServerOpts};
use std::{
    io::{Read, Write},
    net::TcpStream,
    os::unix::fs::PermissionsExt,
};

fn word(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn request(port: u16, procedure: u32, body: &[u8]) -> Vec<u8> {
    rpc(port, 100227, 3, procedure, body)
}

fn rpc(port: u16, program: u32, version: u32, procedure: u32, body: &[u8]) -> Vec<u8> {
    let mut message = vec![];
    for value in [1, 0, 2, program, version, procedure, 0, 0, 0, 0] {
        word(&mut message, value);
    }
    message.extend_from_slice(body);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    send(&mut stream, &message);
    receive(&mut stream)
}

fn send(stream: &mut TcpStream, message: &[u8]) {
    stream
        .write_all(&(0x8000_0000u32 | message.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(message).unwrap();
}

fn receive(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).unwrap();
    let size = (u32::from_be_bytes(header) & 0x7fff_ffff) as usize;
    let mut response = vec![0; size];
    stream.read_exact(&mut response).unwrap();
    response
}

fn value(response: &[u8], position: usize) -> u32 {
    u32::from_be_bytes(response[position..position + 4].try_into().unwrap())
}

#[test]
fn readonly_acl_service_returns_file_permissions_and_rejects_all_writes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    std::fs::write(&file, "source").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o604)).unwrap();
    let server = spawn_nfs_server(&NfsServerOpts {
        export_root: root.path().into(),
        host: "127.0.0.1".into(),
        port: 0,
        read_only: true,
    })
    .unwrap();
    let port = server.local_addr().unwrap().port();
    for program in [100003, 100005, 100227] {
        let mut mapping = vec![];
        for number in [program, 3, 6, 0] {
            word(&mut mapping, number);
        }
        assert_eq!(value(&rpc(port, 100000, 2, 3, &mapping), 24), port as u32);
    }
    assert_eq!(value(&rpc(port, 100227, 2, 0, &[]), 20), 2);
    assert_eq!(value(&request(port, 9, &[]), 20), 3);
    assert_eq!(request(port, 0, &[]).len(), 24);
    let mut body = vec![];
    word(&mut body, 4);
    body.extend_from_slice(b"file");
    word(&mut body, 5);
    let response = request(port, 1, &body);
    assert_eq!(value(&response, 24), 0);
    assert_eq!(value(&response, 116), 5);
    assert_eq!(value(&response, 120), 4);
    assert_eq!(value(&response, 124), 4);
    assert_eq!(value(&response, 136), 6);
    assert_eq!(value(&response, 148), 0);
    assert_eq!(value(&response, 160), 0);
    assert_eq!(value(&response, 172), 4);
    assert_eq!(value(&response, 176), 0);
    assert_eq!(value(&response, 180), 0);
    let response = request(port, 2, &body);
    assert_eq!(value(&response, 24), 30);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "source");
    assert_eq!(
        std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
        0o604
    );
    server.shutdown();
}

#[test]
fn retransmitted_nfs_request_does_not_block_later_nfs_or_acl_replies() {
    let root = tempfile::tempdir().unwrap();
    let server = spawn_nfs_server(&NfsServerOpts {
        export_root: root.path().into(),
        host: "127.0.0.1".into(),
        port: 0,
        read_only: true,
    })
    .unwrap();
    let mut stream = TcpStream::connect(server.local_addr().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let null_call = |xid, program| {
        let mut message = vec![];
        for value in [xid, 0, 2, program, 3, 0, 0, 0, 0, 0] {
            word(&mut message, value);
        }
        message
    };
    send(&mut stream, &null_call(1, 100003));
    assert_eq!(value(&receive(&mut stream), 0), 1);
    // The real backend drops this duplicate without returning a reply.
    send(&mut stream, &null_call(1, 100003));
    send(&mut stream, &null_call(2, 100003));
    send(&mut stream, &null_call(3, 100227));
    let mut replies = [receive(&mut stream), receive(&mut stream)];
    replies.sort_by_key(|reply| value(reply, 0));
    for (reply, xid) in replies.iter().zip([2, 3]) {
        assert_eq!(reply.len(), 24);
        assert_eq!(value(reply, 0), xid);
        assert_eq!(value(reply, 20), 0);
    }
    drop(stream);
    server.shutdown();
}
