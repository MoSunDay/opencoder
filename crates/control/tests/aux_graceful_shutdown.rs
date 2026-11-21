//! Auxiliary resource servers must drain in-flight requests when the
//! process is asked to exit instead of cutting connections mid-reply.
//! Exercises `release::resources::serve_until` with a real listener and a
//! slow handler so the shutdown contract is observed end to end.
use axum::{routing::get, Router};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Minimal HTTP/1.1 GET over a raw stream: one write, read to EOF, then
/// report the status line so keep-alive vs close cannot blur the result.
async fn raw_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("send request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read reply");
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn graceful_shutdown_finishes_inflight_requests_before_stopping() {
    let app = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            "done"
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (fire_shutdown, shutdown) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        opencoder_control::release::resources::serve_until(
            listener,
            app,
            async {
                let _ = shutdown.await;
            },
        )
        .await
    });

    // The slow request is in flight when shutdown fires: graceful drain must
    // still deliver its 200 instead of resetting the connection.
    let reply = tokio::spawn(raw_get(addr, "/slow"));
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(fire_shutdown);

    let body = reply.await.unwrap();
    assert!(body.starts_with("HTTP/1.1 200"), "in-flight reply delivered: {body}");
    assert!(body.ends_with("done"));

    server.await.unwrap().expect("serve_until returns Ok after drain");

    // Once the listener is released, new connections are refused.
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener must stop accepting after shutdown"
    );
}
