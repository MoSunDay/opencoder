mod support;

use opencoder_core::fleet::{
    DetailFieldChunk, DetailFieldRequest, EventPayloadChunk, EventPayloadRequest, ExecutionKind,
    MessageCursor, MessagePage, NodeOperation, QUERY_RESPONSE_BYTES,
};
use opencoder_core::Message;
use opencoder_node::fleet::NodeService;
use opencoder_store::{EventKind, LibsqlStore, SessionEventRecord, Store};
use serde_json::json;
use support::*;

#[tokio::test]
async fn oversized_legacy_message_is_chunked_and_detail_stays_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let node = worker(dir.path(), mock()).await;
    let id = "agent-big-message";
    let definition = json!({"blob":"d".repeat(200_000)});
    node.handle(NodeOperation::Create {
        assignment: assignment(
            &node,
            id,
            ExecutionKind::Agent,
            json!({"prompt":"seed"}),
            Some(definition.clone()),
        ),
    })
    .await;
    settled(&node, id).await;
    let fixture = LibsqlStore::open(dir.path().join("node/runtime.db"))
        .await
        .unwrap();
    let huge = Message::user("legacy-huge", "界".repeat(800_000));
    let huge_seq = fixture.append_message(id, &huge).await.unwrap();
    let execution = node
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == id)
        .unwrap()
        .execution_ref();

    let detail = node
        .handle(NodeOperation::Inspect {
            execution: execution.clone(),
        })
        .await;
    assert_eq!(detail.status, 200, "{detail:?}");
    assert!(serde_json::to_vec(&detail.body).unwrap().len() <= QUERY_RESPONSE_BYTES);
    assert!(detail.body["session"]["messages"]["more"]
        .as_bool()
        .unwrap());
    assert_eq!(detail.body["definition"]["read_via"], "detail_field");

    let mut offset = 0;
    let mut definition_json = Vec::new();
    loop {
        let reply = node
            .handle(NodeOperation::DetailField {
                request: DetailFieldRequest {
                    execution: execution.clone(),
                    field: "definition".into(),
                    offset,
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let chunk: DetailFieldChunk = serde_json::from_value(reply.body).unwrap();
        definition_json.extend(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chunk.bytes_b64)
                .unwrap(),
        );
        if chunk.eof {
            break;
        }
        offset = chunk.next_offset;
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&definition_json).unwrap(),
        definition
    );

    let mut cursor = MessageCursor::default();
    let mut first_json = Vec::new();
    loop {
        let reply = node
            .handle(NodeOperation::Messages {
                execution: execution.clone(),
                cursor,
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        assert!(serde_json::to_vec(&reply.body).unwrap().len() <= QUERY_RESPONSE_BYTES);
        let page: MessagePage = serde_json::from_value(reply.body).unwrap();
        for chunk in page.chunks {
            if chunk.seq == huge_seq {
                first_json.extend(
                    base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        chunk.bytes_b64,
                    )
                    .unwrap(),
                );
            }
        }
        let Some(next) = page.next_cursor else { break };
        cursor = next;
    }
    let blocks: serde_json::Value = serde_json::from_slice(&first_json).unwrap();
    assert_eq!(blocks[0]["text"].as_str().unwrap().chars().count(), 800_000);

    let before = fixture.last_event_seq(id).await.unwrap();
    let event_seq = fixture
        .append_event(&SessionEventRecord {
            session_id: id.into(),
            kind: EventKind::ToolEnd,
            payload: json!({"output":"e".repeat(1_200_000)}),
            ts: 9,
            seq: None,
            sse_kind: Some("tool_end".into()),
        })
        .await
        .unwrap();
    let page = node
        .handle(NodeOperation::Events {
            execution: execution.clone(),
            after: before,
        })
        .await;
    assert_eq!(page.status, 200, "{page:?}");
    assert_eq!(page.body["events"][0]["seq"], event_seq);
    assert_eq!(page.body["events"][0]["data"]["read_via"], "event_payload");
    let mut offset = 0;
    let mut event_json = Vec::new();
    loop {
        let reply = node
            .handle(NodeOperation::EventPayload {
                request: EventPayloadRequest {
                    execution: execution.clone(),
                    seq: event_seq,
                    offset,
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let chunk: EventPayloadChunk = serde_json::from_value(reply.body).unwrap();
        event_json.extend(
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, chunk.bytes_b64)
                .unwrap(),
        );
        if chunk.eof {
            break;
        }
        offset = chunk.next_offset;
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&event_json).unwrap()["output"]
            .as_str()
            .unwrap()
            .len(),
        1_200_000
    );
    node.shutdown().await.unwrap();
}
