//! Manual dispatch of Message-triggered playbooks and the 一期 manual
//! trigger scan (similarity via the mock embedder) + list/get edges.

use opencoder_brain::PlaybookTrigger;
use opencoder_store::BrainPlaybookRecord;
use reqwest::Method;
use serde_json::json;

use super::{agent, seed_playbook, spec};
use crate::support::Harness;

/// Manual dispatch never gates on the trigger: a Message-triggered playbook
/// stays directly dispatchable (auto-fire is the 二期 track).
#[tokio::test]
async fn message_trigger_playbook_is_manually_dispatchable() {
    let h = Harness::new().await;
    seed_playbook(
        &h,
        &spec(
            "playbook-msg",
            PlaybookTrigger::Message {
                match_text: "resize the fleet tonight".into(),
                threshold: 0.9,
            },
            vec![agent("act-on-it")],
        ),
    )
    .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/playbook-msg/dispatch",
            Some(json!({"request_id": "msgreq", "situation": "manual override"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["batches"], json!([["act-on-it"]]));
}

/// The 一期 manual trigger scan: identical texts cosine to ~1.0 and fire;
/// unrelated texts and manual triggers do not. Corrupt specs are skipped.
#[tokio::test]
async fn trigger_scan_reports_matching_message_triggers() {
    let h = Harness::new().await;
    seed_playbook(
        &h,
        &spec(
            "playbook-hot",
            PlaybookTrigger::Message {
                match_text: "resize the fleet tonight".into(),
                threshold: 0.9,
            },
            vec![agent("resize")],
        ),
    )
    .await;
    seed_playbook(
        &h,
        &spec(
            "playbook-cold",
            PlaybookTrigger::Message {
                match_text: "write the quarterly report".into(),
                threshold: 0.9,
            },
            vec![agent("report")],
        ),
    )
    .await;
    seed_playbook(
        &h,
        &spec(
            "playbook-manual",
            PlaybookTrigger::Manual {},
            vec![agent("m")],
        ),
    )
    .await;
    h.state
        .store
        .save_brain_playbook(&BrainPlaybookRecord {
            id: "playbook-corrupt".into(),
            name: "corrupt".into(),
            origin: "fixed".into(),
            situation_digest: None,
            spec_json: "{not json".into(),
            created_at: 0,
            updated_at: 0,
        })
        .await
        .unwrap();

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/trigger-scan",
            Some(json!({"text": "resize the fleet tonight"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let matches = body["matches"].as_array().cloned().unwrap();
    assert_eq!(matches.len(), 1, "{body}");
    assert_eq!(matches[0]["playbook_id"], json!("playbook-hot"));
    assert_eq!(matches[0]["name"], json!("playbook-hot"));
    assert!(matches[0]["similarity"].as_f64().unwrap() > 0.99, "{body}");

    // List/get passthrough sees every persisted record (corrupt included —
    // the store keeps spec_json opaque), and 404s cleanly on misses.
    let (status, body) = h.req(Method::GET, "/api/brain/playbooks", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["playbooks"].as_array().unwrap().len(), 4);
    let (status, body) = h
        .req(Method::GET, "/api/brain/playbooks/playbook-hot", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("playbook-hot"));
    let (status, body) = h
        .req(Method::GET, "/api/brain/playbooks/playbook-none", None)
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/trigger-scan",
            Some(json!({"text": "   "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

/// The scan embeds the incoming text plus every match_text in ONE batched
/// round-trip (`embed_many`, input order preserved) instead of N+1
/// `embed_one` calls — the mock records the call list, so the batching is
/// directly observable while the response shape stays untouched.
#[tokio::test]
async fn trigger_scan_batches_embeds_into_one_round_trip() {
    let h = Harness::new().await;
    seed_playbook(
        &h,
        &spec(
            "playbook-batch-hot",
            PlaybookTrigger::Message {
                match_text: "resize the fleet tonight".into(),
                threshold: 0.9,
            },
            vec![agent("resize")],
        ),
    )
    .await;
    seed_playbook(
        &h,
        &spec(
            "playbook-batch-cold",
            PlaybookTrigger::Message {
                match_text: "write the quarterly report".into(),
                threshold: 0.9,
            },
            vec![agent("report")],
        ),
    )
    .await;
    seed_playbook(
        &h,
        &spec(
            "playbook-batch-manual",
            PlaybookTrigger::Manual {},
            vec![agent("m")],
        ),
    )
    .await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/playbooks/trigger-scan",
            Some(json!({"text": "resize the fleet tonight"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let matches = body["matches"].as_array().cloned().unwrap();
    assert_eq!(matches.len(), 1, "{body}");
    assert_eq!(matches[0]["playbook_id"], json!("playbook-batch-hot"));

    // One embed call: incoming text first, then each parseable Message
    // trigger's match_text in record (store listing) order — the mock
    // records the call list, so the batching is directly observable.
    let calls = h.mock_llm.embed_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0].0,
        vec![
            "resize the fleet tonight".to_string(),
            "write the quarterly report".to_string(),
            "resize the fleet tonight".to_string(),
        ],
        "{calls:?}"
    );
}
