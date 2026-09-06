//! Artifact download streaming (`/api/executions/:id/artifact`): control-side
//! gating, zero/multi-chunk protocol reassembly, filename sanitization, and
//! the inconsistent-metadata 502 / mid-stream abort rules for raw node chunks.

use base64::Engine;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, RpcReply};
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, TOKEN};

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A protocol-consistent chunk reply covering `bytes` at `offset` of `total`.
fn chunk(
    step: &str,
    file: &str,
    offset: u64,
    bytes: &[u8],
    total: u64,
    eof: bool,
    version: &str,
) -> RpcReply {
    RpcReply::ok(json!({
        "step": step, "file": file, "offset": offset,
        "next_offset": offset + bytes.len() as u64, "total_bytes": total,
        "version": version, "eof": eof, "encoding": "base64", "bytes_b64": b64(bytes),
    }))
}

/// Pure copy of `reply` with one JSON field overridden (fault injection).
fn with_field(reply: &RpcReply, field: &str, value: Value) -> RpcReply {
    let mut body = reply.body.clone();
    body[field] = value;
    RpcReply {
        status: reply.status,
        body,
    }
}

/// The raw node table addresses replies by `offset / 65536`, so a two-chunk
/// script always uses a full first chunk plus a small tail.
const SPLIT: usize = 65_536;

fn two_chunk_script(step: &str, file: &str, second: RpcReply) -> Vec<RpcReply> {
    let head = vec![b'A'; SPLIT];
    let total = (SPLIT + 100) as u64;
    vec![chunk(step, file, 0, &head, total, false, "v1"), second]
}

/// Headers are already flushed when a later chunk is rejected: the body must
/// abort short of the advertised total (connection reset or short read).
async fn assert_aborts_short(h: &Harness, path: &str, total: usize) {
    let resp = h.req_raw(Method::GET, path, None, Some(TOKEN)).await;
    assert_eq!(resp.status(), 200, "headers precede the bad chunk");
    if let Ok(bytes) = resp.bytes().await {
        assert!(
            bytes.len() < total,
            "body must stop short of {total} bytes, got {}",
            bytes.len()
        );
    } // an Err (connection reset mid-body) is the same outcome
}

#[tokio::test]
async fn unknown_execution_id_is_a_control_side_404() {
    let h = Harness::new().await;
    // Never registered via put_index/create: rejected before any node call.
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/dag-none-404/artifact?step=build&file=out.txt",
            None,
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("execution id not found"));
}

#[tokio::test]
async fn zero_byte_artifact_streams_an_empty_200_body() {
    let h = Harness::new().await;
    h.put_index("dag-zero-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    h.node
        .set_artifact("dag-zero-1", "build", "empty.bin", Vec::new());
    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-zero-1/artifact?step=build&file=empty.bin",
            None,
            Some(TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-length").unwrap(), "0");
    assert_eq!(
        resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"dag-zero-1-build-empty.bin\""
    );
    assert!(resp.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn two_chunk_artifact_declares_total_length_and_filename() {
    let h = Harness::new().await;
    h.put_index("dag-two-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    // 100_000 bytes forces two 64 KiB-protocol chunks.
    let payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    h.node
        .set_artifact("dag-two-1", "build", "report.bin", payload.clone());
    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-two-1/artifact?step=build&file=report.bin",
            None,
            Some(TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-length").unwrap(), "100000");
    assert_eq!(
        resp.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"dag-two-1-build-report.bin\""
    );
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(bytes.len(), payload.len(), "chunked reassembly length");
    assert_eq!(&bytes[..], &payload[..], "chunked reassembly content");
}

#[tokio::test]
async fn filename_is_sanitized_in_the_disposition_header() {
    let h = Harness::new().await;
    h.put_index("dag-safe-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    h.node
        .set_artifact("dag-safe-1", "build", "a/b c.txt", b"hi".to_vec());
    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-safe-1/artifact?step=build&file=a%2Fb%20c.txt",
            None,
            Some(TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    // '/' and ' ' inside the file name are rewritten to '_' before the
    // header is built; the fixed `attachment; filename="…"` shape is intact.
    let disposition = resp
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        disposition,
        "attachment; filename=\"dag-safe-1-build-a_b_c.txt\""
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), &b"hi"[..]);
}

#[tokio::test]
async fn first_chunk_faults_reject_as_json_502s() {
    let h = Harness::new().await;
    h.put_index("dag-502-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    // Unknown encoding → the inconsistent-metadata branch.
    let bad_encoding = with_field(
        &chunk("build", "enc.bin", 0, b"hi", 2, true, "v1"),
        "encoding",
        json!("plain"),
    );
    h.node
        .set_artifact_raw("dag-502-1", "build", "enc.bin", vec![bad_encoding]);
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/dag-502-1/artifact?step=build&file=enc.bin",
            None,
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(
        body["error"],
        json!("node returned inconsistent artifact metadata")
    );

    // Undecodable base64 → the invalid-base64 branch.
    let bad_b64 = with_field(
        &chunk("build", "b64.bin", 0, b"hi", 2, true, "v1"),
        "bytes_b64",
        json!("###"),
    );
    h.node
        .set_artifact_raw("dag-502-1", "build", "b64.bin", vec![bad_b64]);
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/dag-502-1/artifact?step=build&file=b64.bin",
            None,
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(
        body["error"],
        json!("node returned invalid artifact base64")
    );

    // Missing chunk fields → the invalid-chunk-shape branch.
    h.node.set_artifact_raw(
        "dag-502-1",
        "build",
        "shape.bin",
        vec![RpcReply::ok(json!({"step": "build"}))],
    );
    let (status, body) = h
        .req(
            Method::GET,
            "/api/executions/dag-502-1/artifact?step=build&file=shape.bin",
            None,
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert_eq!(
        body["error"],
        json!("node returned an invalid artifact chunk")
    );
}

#[tokio::test]
async fn later_chunk_faults_abort_the_stream_short_of_total() {
    let h = Harness::new().await;
    h.put_index("dag-abort-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    let total = SPLIT + 100;

    // (b) offset mismatch: the second chunk claims a foreign offset.
    let offset_mismatch = with_field(
        &chunk(
            "build",
            "out.bin",
            SPLIT as u64,
            &[b'B'; 100],
            total as u64,
            true,
            "v1",
        ),
        "offset",
        json!(99),
    );
    h.node.set_artifact_raw(
        "dag-abort-1",
        "build",
        "offset.bin",
        two_chunk_script("build", "offset.bin", offset_mismatch),
    );
    assert_aborts_short(
        &h,
        "/api/executions/dag-abort-1/artifact?step=build&file=offset.bin",
        total,
    )
    .await;

    // (c) version drift between the two chunks.
    let version_drift = with_field(
        &chunk(
            "build",
            "out.bin",
            SPLIT as u64,
            &[b'B'; 100],
            total as u64,
            true,
            "v1",
        ),
        "version",
        json!("v2"),
    );
    h.node.set_artifact_raw(
        "dag-abort-1",
        "build",
        "version.bin",
        two_chunk_script("build", "version.bin", version_drift),
    );
    assert_aborts_short(
        &h,
        "/api/executions/dag-abort-1/artifact?step=build&file=version.bin",
        total,
    )
    .await;

    // (d) next_offset inconsistent with offset + len.
    let next_mismatch = with_field(
        &chunk(
            "build",
            "out.bin",
            SPLIT as u64,
            &[b'B'; 100],
            total as u64,
            true,
            "v1",
        ),
        "next_offset",
        json!((total + 5) as u64),
    );
    h.node.set_artifact_raw(
        "dag-abort-1",
        "build",
        "next.bin",
        two_chunk_script("build", "next.bin", next_mismatch),
    );
    assert_aborts_short(
        &h,
        "/api/executions/dag-abort-1/artifact?step=build&file=next.bin",
        total,
    )
    .await;
}

#[tokio::test]
async fn raw_two_chunk_script_streams_the_reassembled_bytes() {
    let h = Harness::new().await;
    h.put_index("dag-raw-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    let head = vec![b'A'; SPLIT];
    let tail = vec![b'B'; 100];
    let total = (SPLIT + 100) as u64;
    h.node.set_artifact_raw(
        "dag-raw-1",
        "build",
        "out.bin",
        vec![
            chunk("build", "out.bin", 0, &head, total, false, "v1"),
            chunk("build", "out.bin", SPLIT as u64, &tail, total, true, "v1"),
        ],
    );
    let resp = h
        .req_raw(
            Method::GET,
            "/api/executions/dag-raw-1/artifact?step=build&file=out.bin",
            None,
            Some(TOKEN),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-length")
            .unwrap()
            .to_str()
            .unwrap(),
        "65636"
    );
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(bytes.len(), SPLIT + 100);
    assert_eq!(&bytes[..SPLIT], &head[..]);
    assert_eq!(&bytes[SPLIT..], &tail[..]);
}
