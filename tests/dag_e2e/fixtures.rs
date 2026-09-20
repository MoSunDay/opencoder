//! Wasm-step fixtures shared by the DAG e2e scenarios: tiny WASI modules
//! compiled from wat on the fly (`wat` is a dev-dependency), plus the
//! stdlib base64 encoder the `/api/dag/wasm` upload contract needs.

/// Wat for a module that writes `message` (plus newline) to stdout — the
/// captured bytes become the step's `output.txt`.
pub fn stdout_module_wat(message: &str) -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{msg}\n")
  (func (export "_start")
    (i32.store (i32.const 64) (i32.const 0))
    (i32.store (i32.const 68) (i32.const {len}))
    (drop (call $fd_write (i32.const 1) (i32.const 64) (i32.const 1) (i32.const 72)))))"#,
        msg = message,
        len = message.len() + 1
    )
}

/// Wat for a module that dumps its whole WASI argv buffer to stdout: every
/// argument NUL-separated, argv[0] first (the module token). The
/// input-args e2e asserts dispatch `input.args` shows up as command-line
/// tokens after the module token.
pub fn args_echo_wat() -> String {
    r#"(module
  (import "wasi_snapshot_preview1" "args_sizes_get"
    (func $args_sizes_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "args_get"
    (func $args_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "_start")
    ;; Layout: [0]=argc, [4]=buf size, [8..]=argv pointers, [4096..]=the
    ;; packed NUL-terminated strings. One iovec at 2048 covers the whole
    ;; buffer, so stdout is argv joined by NUL bytes.
    (drop (call $args_sizes_get (i32.const 0) (i32.const 4)))
    (drop (call $args_get (i32.const 8) (i32.const 4096)))
    (i32.store (i32.const 2048) (i32.const 4096))
    (i32.store (i32.const 2052) (i32.load (i32.const 4)))
    (drop (call $fd_write (i32.const 1) (i32.const 2048) (i32.const 1) (i32.const 2056)))))"#
        .to_string()
}

/// Wat for an endless loop — terminated only by cancellation (epoch
/// interruption) or the step timeout.
pub const SPIN_WAT: &str = r#"(module (func (export "_start") (loop $l (br $l))))"#;

/// Compile `wat` source to wasm bytes.
pub fn compile(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("wat fixture must compile")
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Minimal standard base64 encoder (padded): the pool API takes
/// `wasm_b64`, and the root package has no base64 dev-dependency.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let word = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[(word >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(word >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64_ALPHABET[(word >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64_ALPHABET[word as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The `/api/dag/wasm` create body for `name` serving `wat`.
pub fn pool_create_body(name: &str, description: &str, wat: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "wasm_b64": base64_encode(&compile(wat)),
    })
}

/// Publish `name` at v1 through the real pool API; returns the version.
pub fn publish(fleet: &crate::support::fleet_proc::Fleet, name: &str, wat: &str) -> u32 {
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/wasm",
        &pool_create_body(name, "e2e fixture", wat),
    );
    assert_eq!(status, 201, "pool publish {name}: {body}");
    body["version"].as_u64().unwrap_or(0) as u32
}

/// Raw binary HTTP GET (Content-Length framed) — for the wasm download
/// endpoint, whose body is arbitrary module bytes, not JSON.
#[allow(dead_code)]
pub fn download_bytes(base: &str, path: &str, token: &str) -> (u16, Vec<u8>) {
    use std::io::{Read, Write};
    let host = base.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(host).expect("connect for download");
    let request = format!(
        "GET {path} HTTP/1.1\r\nhost: {host}\r\nauthorization: Bearer {token}\r\nconnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read response");
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header terminator");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let len: usize = head
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(raw.len() - split - 4);
    (status, raw[split + 4..split + 4 + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-rolled encoder must agree with the reference vectors.
    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
