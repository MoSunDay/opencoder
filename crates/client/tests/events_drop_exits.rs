//! Integration test for P2#12: dropping the `mpsc::Receiver` returned by
//! `Remote::events()` must promptly tear down the spawned streaming task and
//! the underlying HTTP connection.
//!
//! Background: `Remote::events()` spawns a task that drives `run_stream`. That
//! task runs a `tokio::select!` biased toward `tx.closed()` — i.e. once ALL
//! receivers go away, the `closed()` future resolves, the select arm returns
//! `Ok(())`, the task exits, the `reqwest` response stream is dropped, and the
//! server-side socket observes EOF (a clean disconnect). Without this guard
//! the task would keep reading from the SSE stream and the HTTP connection
//! would linger until the server itself closed it (here, never — it stalls).
//!
//! This test stands up a mock SSE server that sends a valid 200 + a
//! keep-alive comment, then STALLS (sends nothing more, holds the socket).
//! The client connects, enters the streaming loop, and then we drop the
//! receiver. We assert the server observes `read() -> Ok(0)` (EOF) quickly —
//! proving the connection was closed on drop rather than lingering.

use std::time::Duration;

use opencoder_client::Remote;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Maximum time the server will wait to observe the client disconnect. If the
/// fix works the EOF arrives within milliseconds of `drop(rx)`; this 3s window
/// is just a generous ceiling so a regression still fails fast-ish.
const SERVER_DISCONNECT_WINDOW: Duration = Duration::from_secs(3);

/// Time the test gives the spawned `events()` task to (a) open the HTTP
/// connection, (b) receive the response headers, and (c) enter the streaming
/// select loop, before we drop the receiver. Kept generous to avoid flakes on
/// busy CI; the assertion itself does not depend on this being tight.
const SETTLE_TIME: Duration = Duration::from_millis(150);

/// Upper bound for receiving the server's verdict via the oneshot channel.
/// Deliberately *shorter* than `SERVER_DISCONNECT_WINDOW`: with the fix the
/// EOF (and thus the verdict) arrives within milliseconds of `drop(rx)`, well
/// inside this window. A regression — where the task lingers — would not
/// produce a verdict in time, so this timeout makes the test fail fast with a
/// clear message instead of waiting out the full server window.
const VERDICT_TIMEOUT: Duration = Duration::from_secs(2);

/// Start a mock SSE server that accepts one connection, replies with valid
/// SSE headers plus a keep-alive comment frame, then stalls. It then waits
/// (up to `SERVER_DISCONNECT_WINDOW`) to see the client disconnect as an EOF.
///
/// Returns the base URL (`http://127.0.0.1:PORT`) the client should target and
/// a oneshot receiver carrying `true` if an EOF was observed (good) or `false`
/// otherwise (timeout / error / unexpected data — a regression).
async fn start_stall_server() -> (String, tokio::sync::oneshot::Receiver<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();

        // Consume the HTTP request line + headers. A single read is enough to
        // drain the headers for these tiny requests; we don't need a body.
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf).await;

        // Valid SSE response + a keep-alive comment frame, then stall: send no
        // more data and hold the socket open. The client's `run_stream` will be
        // parked in its `select!` waiting on `stream.next()`.
        let resp = b"HTTP/1.1 200 OK\r\n\
                     Content-Type: text/event-stream\r\n\
                     Cache-Control: no-cache\r\n\
                     Connection: keep-alive\r\n\
                     \r\n\
                     :keep-alive\n\n";
        sock.write_all(resp).await.unwrap();

        // Wait for the client to disconnect. With the `tx.closed()` guard the
        // task exits promptly on `drop(rx)` → the reqwest response is dropped →
        // the TCP connection closes → this `read` returns `Ok(0)` (EOF).
        // Without the guard the task lingers and this read hits the 3s timeout.
        let got_eof =
            match tokio::time::timeout(SERVER_DISCONNECT_WINDOW, sock.read(&mut buf)).await {
                Ok(Ok(0)) => true,
                // Any non-zero bytes or a read error still indicate the connection
                // was disturbed; but a healthy fix yields a clean EOF, so treat
                // everything else as "did not promptly disconnect".
                Ok(Ok(_)) | Ok(Err(_)) | Err(_) => false,
            };
        // Best-effort: try to flush any RST, ignore failure.
        let _ = sock.shutdown().await;
        let _ = tx.send(got_eof);
    });
    (format!("http://{}", addr), rx)
}

/// Dropping the receiver returned by `Remote::events()` must cause the spawned
/// streaming task to exit promptly via the `tx.closed()` select arm, which in
/// turn closes the HTTP connection. The mock server should therefore observe
/// an EOF shortly after `drop(rx)`, not a multi-second stall/timeout.
#[tokio::test]
async fn dropping_receiver_prompts_stream_task_exit() {
    let (url, server_rx) = start_stall_server().await;

    let remote = Remote::new(&url, "tok").expect("Remote::new");
    let rx = remote.events("s1", 0).expect("events receiver");

    // Let the spawned task connect, receive headers, and enter the streaming
    // select loop before we pull the rug out.
    tokio::time::sleep(SETTLE_TIME).await;

    // The trigger: drop the only receiver. `tx.closed()` should now resolve.
    drop(rx);

    // The server's verdict: did it see a prompt EOF?
    let got_eof = tokio::time::timeout(VERDICT_TIMEOUT, server_rx)
        .await
        .expect("server did not report a verdict within the timeout")
        .expect("server result channel was dropped unexpectedly");

    assert!(
        got_eof,
        "server should observe EOF — the HTTP connection must close promptly \
         after the receiver is dropped (the streaming task's `tx.closed()` \
         guard must exit it and drop the reqwest response)"
    );
}
