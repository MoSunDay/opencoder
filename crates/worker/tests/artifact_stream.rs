mod support;

use futures::StreamExt;
use opencoder_core::fleet::{
    ArtifactChunk, ArtifactRequest, ExecutionKind, NodeOperation, ARTIFACT_CHUNK_BYTES,
    MAX_FRAME_BYTES,
};
use opencoder_node::fleet::NodeService;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Seek, Write};
use support::*;

const FIXTURE_BYTES: u64 = 256 * 1024 * 1024;

fn rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        })
        .unwrap_or(0)
        * 1024
}

#[tokio::test]
async fn streams_256_mib_artifact_with_bounded_frames_and_memory() {
    let fleet = Fleet::new(1, mock()).await;
    let spec = json!({"name":"large-dag","steps":[{
        "name":"first","kind":{"type":"python","code":"print('small')"}
    }]});
    let assignment = assignment(
        &fleet.nodes[0],
        "dag-large-artifact",
        ExecutionKind::Dag,
        json!({}),
        Some(spec),
    );
    let index = assignment.index.clone();
    assert_eq!(
        fleet.nodes[0]
            .handle(NodeOperation::Create { assignment })
            .await
            .status,
        200
    );
    settled(&fleet.nodes[0], "dag-large-artifact").await;
    fleet.state.fleet.put_index(&index).await.unwrap();

    let path = fleet
        .root()
        .join("n0/node/dag/dag-large-artifact/first/output.txt");
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap()
        .set_len(FIXTURE_BYTES)
        .unwrap();

    let page = fleet.nodes[0]
        .handle(NodeOperation::Artifact {
            request: ArtifactRequest {
                execution: index.execution_ref(),
                step: "first".into(),
                file: "output.txt".into(),
                offset: 0,
                version: None,
            },
        })
        .await;
    assert_eq!(page.status, 200, "{page:?}");
    assert!(serde_json::to_vec(&page).unwrap().len() < MAX_FRAME_BYTES);
    let first: ArtifactChunk = serde_json::from_value(page.body).unwrap();

    let response = fleet
        .response(
            "GET",
            "/api/executions/dag-large-artifact/artifact?step=first&file=output.txt",
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-length"].to_str().unwrap(),
        FIXTURE_BYTES.to_string().as_str()
    );
    let baseline_rss = rss_bytes();
    let mut peak_rss = baseline_rss;
    let mut total = 0u64;
    let mut checksum = Sha256::new();
    let mut stream = response.into_body().into_data_stream();
    while let Some(frame) = stream.next().await {
        let frame = frame.unwrap();
        assert!(frame.len() <= ARTIFACT_CHUNK_BYTES);
        total += frame.len() as u64;
        checksum.update(&frame);
        peak_rss = peak_rss.max(rss_bytes());
    }
    assert_eq!(total, FIXTURE_BYTES);

    let zeros = [0u8; ARTIFACT_CHUNK_BYTES];
    let mut expected = Sha256::new();
    for _ in 0..FIXTURE_BYTES / ARTIFACT_CHUNK_BYTES as u64 {
        expected.update(zeros);
    }
    assert_eq!(checksum.finalize(), expected.finalize());
    assert!(
        peak_rss.saturating_sub(baseline_rss) < 64 * 1024 * 1024,
        "streaming grew RSS by {} bytes",
        peak_rss.saturating_sub(baseline_rss)
    );

    let mut changed = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    changed.seek(std::io::SeekFrom::Start(0)).unwrap();
    changed.write_all(&[1]).unwrap();
    changed.sync_all().unwrap();
    let stale = fleet.nodes[0]
        .handle(NodeOperation::Artifact {
            request: ArtifactRequest {
                execution: index.execution_ref(),
                step: "first".into(),
                file: "output.txt".into(),
                offset: first.next_offset,
                version: Some(first.version),
            },
        })
        .await;
    assert_eq!(stale.status, 409, "{stale:?}");
    fleet.shutdown().await;
}
