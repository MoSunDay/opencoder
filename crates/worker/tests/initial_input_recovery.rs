mod support;

use opencoder_core::{fleet::*, Role};
use opencoder_node::fleet::NodeService;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use serde_json::{json, Value};
use support::{assignment, mock, settled, worker};

#[derive(Clone, Copy)]
enum Seed {
    NoSession,
    EmptySession,
    Admitted,
    Promoted,
}

fn journal_path(root: &std::path::Path, id: &str) -> std::path::PathBuf {
    root.join("node/agent").join(id).join("execution.json")
}

fn write_accepted(root: &std::path::Path, assignment: &Assignment) {
    let path = journal_path(root, &assignment.index.id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&json!({
            "assignment": assignment,
            "result": null,
            "error": null,
            "events": [],
        }))
        .unwrap(),
    )
    .unwrap();
}

async fn seed_session(root: &std::path::Path, id: &str, prompt: &str, seed: Seed) {
    if matches!(seed, Seed::NoSession) {
        return;
    }
    let store = LibsqlStore::open(root.join("node/runtime.db"))
        .await
        .unwrap();
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some(prompt.into()),
            agent: Some("act".into()),
            model: None,
            autopilot_mode: None,
            workdir_hash: None,
            created_at: 1,
            updated_at: 1,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    if matches!(seed, Seed::Admitted | Seed::Promoted) {
        let outcome = store
            .admit_input_once(&SessionInput {
                seq: None,
                id: format!("initial-{id}"),
                session_id: id.into(),
                delivery: Delivery::Steer,
                prompt: prompt.into(),
                images: vec![],
                display_text: None,
                admitted_seq: 0,
                promoted_seq: None,
            })
            .await
            .unwrap();
        assert!(outcome.inserted);
        if matches!(seed, Seed::Promoted) {
            let promoted = store
                .promote_inputs(id, outcome.seq, Delivery::Steer)
                .await
                .unwrap();
            assert_eq!(promoted, vec![outcome.seq]);
        }
    }
}

async fn resume(root: &std::path::Path, id: &str, prompt: &str, seed: Seed) {
    let client = mock();
    let bootstrap = worker(root, client.clone()).await;
    let assignment = assignment(
        &bootstrap,
        id,
        ExecutionKind::Agent,
        json!({"prompt":prompt,"title":"recovery"}),
        None,
    );
    write_accepted(root, &assignment);
    drop(bootstrap);
    seed_session(root, id, prompt, seed).await;

    let recovered = worker(root, client.clone()).await;
    let inspected = recovered
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Agent,
            },
        })
        .await;
    assert_eq!(inspected.body["execution"]["status"], "interrupted");
    let started = recovered
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Agent,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(started.status, 200, "{started:?}");
    let detail = settled(&recovered, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert_eq!(client.call_count(), 1, "initial input must execute once");
    recovered.shutdown().await.unwrap();
    drop(recovered);

    let store = LibsqlStore::open(root.join("node/runtime.db"))
        .await
        .unwrap();
    let messages = store.load_messages(id).await.unwrap();
    let users: Vec<_> = messages
        .iter()
        .filter(|message| message.role == Role::User)
        .collect();
    assert_eq!(users.len(), 1, "one durable user prompt: {messages:?}");
    assert!(users[0]
        .blocks
        .iter()
        .any(|block| block.as_text() == Some(prompt)));
}

#[tokio::test]
async fn recovery_admits_once_from_every_pre_execution_fault_point() {
    let cases = [
        ("no-session", Seed::NoSession),
        ("empty-session", Seed::EmptySession),
        ("already-admitted", Seed::Admitted),
        ("promoted-not-recorded", Seed::Promoted),
    ];
    for (name, seed) in cases {
        let dir = tempfile::tempdir().unwrap();
        resume(dir.path(), name, "recover this prompt", seed).await;
    }

    // Fleet ids allow 64 bytes. The derived `initial-` key is 72 bytes and
    // must remain a store idempotency key rather than pass the fleet-id limit.
    let dir = tempfile::tempdir().unwrap();
    let max_id = "x".repeat(64);
    resume(dir.path(), &max_id, "maximum id prompt", Seed::NoSession).await;
}

#[tokio::test]
async fn resume_after_completed_first_turn_does_not_call_the_model_again() {
    let dir = tempfile::tempdir().unwrap();
    let client = mock();
    let first = worker(dir.path(), client.clone()).await;
    let id = "agent-completed-first-turn";
    let accepted = first
        .handle(NodeOperation::Create {
            assignment: assignment(
                &first,
                id,
                ExecutionKind::Agent,
                json!({"prompt":"only once","title":"completed"}),
                None,
            ),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let detail = settled(&first, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert_eq!(client.call_count(), 1);
    first.shutdown().await.unwrap();
    drop(first);

    let path = journal_path(dir.path(), id);
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["assignment"]["index"]["status"] = json!("running");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();

    let second = worker(dir.path(), client.clone()).await;
    let started = second
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Agent,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await;
    assert_eq!(started.status, 200, "{started:?}");
    let detail = settled(&second, id).await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    assert_eq!(
        client.call_count(),
        1,
        "completed prompt must not re-execute"
    );
    second.shutdown().await.unwrap();
}
