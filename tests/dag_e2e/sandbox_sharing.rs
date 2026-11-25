//! The sandbox isolates processes but shares what a run needs: the steps of
//! one run live in the same PID namespace (distinct from the node's), see the
//! same writable `/workspace`, and reach the node's network because no
//! network namespace is created. The same scenario runs on both backends.

use serde_json::json;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::fixtures::publish;
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;

/// Upper bound on the responder's wait for the step's connection: above
/// `Fleet::wait_terminal`'s own wait, so a run that never reaches the writer
/// step leaves no thread blocked for good without the responder giving up
/// before the harness does.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(300);

/// Pause between two nonblocking `accept` attempts (`TcpListener` has no
/// read-timeout setter in std, so the bound is enforced by polling).
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Waits at most `limit` for one connection on a nonblocking listener and
/// gives up with `None` on timeout or any other error.
fn accept_within(server: &TcpListener, limit: Duration) -> Option<TcpStream> {
    let deadline = Instant::now() + limit;
    loop {
        match server.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(ACCEPT_POLL_INTERVAL);
    }
}

/// Loopback responder proving the sandbox shares the node's network: a step
/// connects to it and reads the fixed reply until EOF.
fn responder() -> u16 {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    server
        .set_nonblocking(true)
        .expect("responder nonblocking accept");
    let port = server.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Some(mut stream) = accept_within(&server, ACCEPT_TIMEOUT) else {
            return;
        };
        stream.write_all(b"loopback-ok").unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
    });
    port
}

fn writer_source(port: u16) -> String {
    format!(
        r#"
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>
int main(void) {{
    FILE *file = fopen("/workspace/netfs-handoff.txt", "w");
    if (!file) {{ perror("workspace handoff"); return 2; }}
    fputs("handoff-ok", file);
    fclose(file);
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) {{ perror("socket"); return 3; }}
    struct sockaddr_in address = {{0}};
    address.sin_family = AF_INET;
    address.sin_port = htons({port});
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (connect(fd, (struct sockaddr *)&address, sizeof address) != 0) {{
        perror("connect loopback"); return 4;
    }}
    char reply[64] = {{0}};
    size_t used = 0;
    while (used < sizeof reply - 1) {{
        ssize_t count = read(fd, reply + used, sizeof reply - 1 - used);
        if (count <= 0) {{ break; }}
        used += (size_t)count;
    }}
    close(fd);
    char ns[64] = {{0}};
    ssize_t links = readlink("/proc/self/ns/pid", ns, sizeof ns - 1);
    ns[links > 0 ? (size_t)links : 0] = 0;
    printf("net:%s\npidns:%s\n", used > 0 ? reply : "empty", ns);
    return 0;
}}
"#
    )
}

const READER_C: &str = r#"
#include <stdio.h>
#include <unistd.h>
int main(void) {
    FILE *file = fopen("/workspace/netfs-handoff.txt", "r");
    if (!file) { perror("workspace handoff"); return 2; }
    char line[64] = {0};
    fgets(line, sizeof line, file);
    fclose(file);
    char ns[64] = {0};
    ssize_t links = readlink("/proc/self/ns/pid", ns, sizeof ns - 1);
    ns[links > 0 ? (size_t)links : 0] = 0;
    printf("fs:%s\npidns:%s\n", line, ns);
    return 0;
}
"#;

fn field(output: &str, key: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap_or_else(|| panic!("{key} missing from: {output}"))
        .to_owned()
}

#[test]
fn steps_share_the_workspace_and_the_node_network() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "sandbox-sharing-node");
    let port = responder();
    publish(&fleet, "netfs-writer", &writer_source(port));
    publish(&fleet, "netfs-reader", READER_C);
    let definition = "e2e-sandbox-sharing";
    let run = "dag-e2e-sandbox-sharing-1";
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/defs",
        &json!({"spec": {"name": definition, "steps": [
            {"name": "writer", "kind": {"type":"binary", "resource":"netfs-writer"}},
            {"name": "reader", "depends_on": ["writer"],
             "kind": {"type":"binary", "resource":"netfs-reader"}},
        ]}}),
    );
    assert_eq!(status, 200, "save def: {body}");
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{definition}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");

    let run_root = fleet.run_root(run);
    let writer = std::fs::read_to_string(run_root.join("writer/output.txt"))
        .unwrap_or_else(|error| panic!("read writer output: {error}"));
    let reader = std::fs::read_to_string(run_root.join("reader/output.txt"))
        .unwrap_or_else(|error| panic!("read reader output: {error}"));
    assert_eq!(field(&writer, "net:"), "loopback-ok", "writer: {writer}");
    assert_eq!(field(&reader, "fs:"), "handoff-ok", "reader: {reader}");

    // One PID namespace for the whole run, isolated from the node's.
    let writer_ns = field(&writer, "pidns:");
    assert_eq!(
        writer_ns,
        field(&reader, "pidns:"),
        "steps left the run namespace"
    );
    let host = std::fs::read_link("/proc/self/ns/pid").unwrap();
    assert_ne!(
        writer_ns,
        host.to_str().unwrap(),
        "the run uses the host PID namespace"
    );

    // The handoff is persisted in the run's workspace write layer: the
    // storage below the sandbox is untouched.
    let handoff = std::fs::read_to_string(run_root.join("upper/netfs-handoff.txt"))
        .unwrap_or_else(|error| panic!("read workspace upper: {error}"));
    assert_eq!(handoff, "handoff-ok");
    assert_eq!(stub.request_count(), 0);
}
