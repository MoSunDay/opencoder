pub fn stdout_source(message: &str) -> String {
    let message = serde_json::to_string(message).unwrap();
    format!("#include <stdio.h>\nint main(void) {{ puts({message}); return 0; }}")
}
pub fn args_source() -> String {
    "#include <stdio.h>\n#include <string.h>\nint main(int argc, char **argv) { for (int index=0; index<argc; index++) fwrite(argv[index],1,strlen(argv[index])+1,stdout); return 0; }".into()
}
pub const SPIN_C: &str = "#include <unistd.h>\nint main(void) { for (;;) pause(); }";

pub fn compile(source: &str) -> Vec<u8> {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("fixture.c");
    let binary = temp.path().join("fixture");
    std::fs::write(&input, source).unwrap();
    let status = std::process::Command::new("cc")
        .args(["-O2", "-static", "-s", "-Wl,--build-id=none"])
        .arg(input)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read(binary).unwrap()
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Minimal standard base64 encoder (padded): the pool API takes
/// `binary_b64`, and the root package has no base64 dev-dependency.
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

/// The `/api/dag/binaries` create body for a compiled Linux resource.
pub fn pool_create_body(name: &str, description: &str, source: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "binary_b64": base64_encode(&compile(source)),
    })
}

/// Publish `name` at v1 through the real pool API; returns the version.
pub fn publish(fleet: &crate::support::fleet_proc::Fleet, name: &str, source: &str) -> u32 {
    let (status, body) = fleet.http(
        "POST",
        "/api/dag/binaries",
        &pool_create_body(name, "e2e fixture", source),
    );
    assert_eq!(status, 201, "pool publish {name}: {body}");
    body["version"].as_u64().unwrap_or(0) as u32
}

/// Raw binary HTTP GET (Content-Length framed) — for the binary download
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
