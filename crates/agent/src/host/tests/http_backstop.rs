//! Host HTTP client backstop behavior: a stalled server must fail through
//! the builder-level timeout, and per-request timeouts must override it so
//! the production call-site budgets keep winning.
use super::super::client::{http_client_with, HTTP_BACKSTOP};
use serde_json::json;
use std::time::{Duration, Instant};

/// Accept connections but never answer: any request must run into a
/// timeout, not a connection error.
async fn stalling_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // Hold every accepted socket open without ever writing a response,
        // so the client can only fail through a timeout.
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    format!("http://{addr}/rpc")
}

#[test]
fn backstop_is_wider_than_every_call_site_budget() {
    // Call-site budgets: 10-45s wrapping timeouts (admission), 15s
    // per-request timeouts (inventory, runtime probes).
    assert!(HTTP_BACKSTOP > Duration::from_secs(45));
}

#[tokio::test]
async fn builder_backstop_times_out_a_stalled_server() {
    let endpoint = stalling_server().await;
    let client = http_client_with(Duration::from_millis(300)).unwrap();
    let started = Instant::now();
    let error = client
        .post(&endpoint)
        .json(&json!({}))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_timeout(), "backstop must surface as a timeout: {error}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "backstop fired near the configured duration, not after a hang"
    );
}

#[tokio::test]
async fn per_request_timeout_overrides_the_builder_backstop() {
    let endpoint = stalling_server().await;
    // Production shape: the 300s backstop underneath, a tight per-request
    // budget on top. The request-level value must win.
    let client = http_client_with(HTTP_BACKSTOP).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        let started = Instant::now();
        let error = client
            .post(&endpoint)
            .timeout(Duration::from_millis(300))
            .json(&json!({}))
            .send()
            .await
            .unwrap_err();
        (error, started.elapsed())
    })
    .await;
    let (error, elapsed) = outcome.expect("the per-request timeout must override the backstop");
    assert!(error.is_timeout(), "override must surface as a timeout: {error}");
    assert!(
        elapsed < Duration::from_secs(3),
        "request finished on its own budget, not on the 300s backstop"
    );
}
