//! HTTP/1.1 liveness probe: plain `http://` only, bounded attempts,
//! no TLS stack in the node runtime.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Pause between probe attempts.
const PROBE_RETRY_PAUSE_MS: u64 = 250;
/// Bound on one probe response read (only the status line matters).
const PROBE_READ_CAP: usize = 8 * 1024;

/// Probe result: the URL is not a plain `http://host[:port]/path`.
pub const PROBE_ERR_URL: i32 = -1;
/// Probe result: every attempt failed before an HTTP response (connect
/// refused / timed out / IO error).
pub const PROBE_ERR_UNREACHABLE: i32 = -2;
/// Probe result: HTTP responses arrived but none matched `expect`.
pub const PROBE_ERR_STATUS: i32 = -3;
/// Probe result: the step was cancelled mid-window.
pub const PROBE_ERR_CANCELLED: i32 = -4;

/// One `(host, port, path)` HTTP/1.1 probe target. `http://` only — the
/// node runtime deliberately ships no TLS stack; anything else is a URL
/// error, not a silent fallback.
pub(crate) struct ProbeTarget {
    pub addr: SocketAddr,
    pub path: String,
    pub host_header: String,
}

pub(crate) fn parse_http_url(url: &str) -> Option<ProbeTarget> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let authority = authority.split('@').next_back()?; // drop userinfo if any
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    Some(ProbeTarget {
        addr,
        path,
        host_header: authority.to_string(),
    })
}

/// Poll `url` until it answers `expect` (or 2xx when `expect <= 0`).
/// Runs on the guest's blocking worker; every attempt is bounded by
/// `timeout_ms`, the window by `retries` extra attempts.
pub(crate) fn http_probe(
    url: &str,
    expect: i32,
    timeout_ms: i32,
    retries: i32,
    cancel: &AtomicBool,
) -> i32 {
    let Some(target) = parse_http_url(url) else {
        return PROBE_ERR_URL;
    };
    let timeout = Duration::from_millis(timeout_ms.max(1) as u64);
    let attempts = retries.max(0) as u64 + 1;
    let mut saw_response = false;
    for _ in 0..attempts {
        if cancel.load(Ordering::SeqCst) {
            return PROBE_ERR_CANCELLED;
        }
        if let Some(code) = probe_once(&target, timeout) {
            saw_response = true;
            let accepted = if expect > 0 {
                i32::from(code) == expect
            } else {
                (200..300).contains(&code)
            };
            if accepted {
                return code as i32;
            }
        }
        std::thread::sleep(Duration::from_millis(PROBE_RETRY_PAUSE_MS));
    }
    if saw_response {
        PROBE_ERR_STATUS
    } else {
        PROBE_ERR_UNREACHABLE
    }
}

/// One bounded HTTP/1.1 GET; `Some(status)` when a status line arrived.
fn probe_once(target: &ProbeTarget, timeout: Duration) -> Option<u16> {
    let mut stream = TcpStream::connect_timeout(&target.addr, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!(
        "GET {} HTTP/1.1\r\nhost: {}\r\nuser-agent: opencode-dag-probe\r\nconnection: close\r\n\r\n",
        target.path, target.host_header
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() >= PROBE_READ_CAP {
                    break;
                }
            }
        }
    }
    let head = String::from_utf8_lossy(&raw);
    head.lines()
        .next()
        .and_then(|status_line| status_line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
}
