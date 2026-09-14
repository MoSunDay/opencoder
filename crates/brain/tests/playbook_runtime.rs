//! Runtime CRUD integration tests for playbooks: create/update/get/list/
//! delete over a real `LibsqlStore` (in-memory) plus `MockChatClient` — the
//! embed path is never exercised, so zero tokens, zero network. The pure
//! domain contract itself is covered by `playbook.rs`.

use std::sync::Arc;

use opencoder_brain::playbook::{
    spec, PlaybookInput, PlaybookOrigin, PlaybookSpec, PlaybookStep, PlaybookTarget,
    PlaybookTrigger,
};
use opencoder_brain::Runtime;
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_store::{BrainPlaybookRecord, LibsqlStore, Store};

fn agent_step(name: &str, deps: &[&str]) -> PlaybookStep {
    PlaybookStep {
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        target: PlaybookTarget::Agent {
            agent: "act".into(),
        },
        prompt: format!("handle {{situation}} for {name}"),
    }
}

fn spec_with_steps(name: &str, steps: Vec<PlaybookStep>) -> PlaybookSpec {
    PlaybookSpec {
        schema_version: spec::SCHEMA_VERSION,
        id: "playbook-test".into(),
        name: name.into(),
        origin: PlaybookOrigin::Fixed {},
        trigger: PlaybookTrigger::Manual {},
        steps,
    }
}

fn input(name: &str, steps: Vec<PlaybookStep>) -> PlaybookInput {
    PlaybookInput {
        name: name.into(),
        trigger: PlaybookTrigger::Manual {},
        steps,
    }
}

async fn setup() -> (Runtime, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    (Runtime::new(store.clone(), client, "mock-embed"), store)
}

/// Validate → the joined error string (panics when the spec unexpectedly

#[tokio::test]
async fn runtime_playbook_crud_roundtrip() {
    let (rt, store) = setup().await;
    let created = rt
        .create_playbook(
            &input(
                "flake fix",
                vec![agent_step("a", &[]), agent_step("b", &["a"])],
            ),
            1_000,
        )
        .await
        .unwrap();
    assert!(created.id.starts_with("playbook-"));
    assert!(matches!(created.origin, PlaybookOrigin::Fixed {}));
    assert_eq!(
        rt.get_playbook_spec(&created.id).await.unwrap().unwrap(),
        created
    );
    let record = rt.get_playbook(&created.id).await.unwrap().unwrap();
    assert_eq!(record.origin, "fixed");
    assert_eq!(record.situation_digest, None);
    assert_eq!(record.created_at, 1_000);

    let second = rt
        .create_playbook(&input("other", vec![agent_step("a", &[])]), 2_000)
        .await
        .unwrap();
    let list = rt.list_playbooks().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, second.id, "newest first");
    assert_eq!(list[1].id, created.id);

    // Update rewrites name + steps, preserves id/origin/digest/created_at.
    let updated = rt
        .update_playbook(
            &created.id,
            &input(
                "renamed",
                vec![
                    agent_step("a", &[]),
                    agent_step("b", &["a"]),
                    agent_step("c", &["a"]),
                ],
            ),
            3_000,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.name, "renamed");
    assert_eq!(updated.steps.len(), 3);
    let record = rt.get_playbook(&created.id).await.unwrap().unwrap();
    assert_eq!(record.created_at, 1_000, "upsert preserves created_at");
    assert_eq!(record.updated_at, 3_000);
    assert_eq!(record.origin, "fixed");
    assert_eq!(
        rt.get_playbook_spec(&created.id)
            .await
            .unwrap()
            .unwrap()
            .steps
            .len(),
        3
    );

    assert!(rt.delete_playbook(&created.id).await.unwrap());
    assert!(
        !rt.delete_playbook(&created.id).await.unwrap(),
        "second delete: no row"
    );
    assert!(rt.get_playbook(&created.id).await.unwrap().is_none());
    assert_eq!(rt.list_playbooks().await.unwrap().len(), 1);

    // Unknown id: Ok(None), nothing touched.
    let none = rt
        .update_playbook(
            "playbook-missing",
            &input("x", vec![agent_step("a", &[])]),
            9_000,
        )
        .await
        .unwrap();
    assert!(none.is_none());
    assert!(store
        .get_brain_playbook("playbook-missing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn update_playbook_preserves_dynamic_origin_and_digest() {
    let (rt, store) = setup().await;
    let spec = PlaybookSpec {
        schema_version: spec::SCHEMA_VERSION,
        id: "playbook-dyn".into(),
        name: "dyn".into(),
        origin: PlaybookOrigin::Dynamic {
            situation_digest: "dig".into(),
            plan_id: Some("brain-plan-1".into()),
        },
        trigger: PlaybookTrigger::Manual {},
        steps: vec![agent_step("a", &[])],
    };
    store
        .save_brain_playbook(&BrainPlaybookRecord {
            id: spec.id.clone(),
            name: spec.name.clone(),
            origin: "dynamic".into(),
            situation_digest: Some("dig".into()),
            spec_json: serde_json::to_string(&spec).unwrap(),
            created_at: 500,
            updated_at: 500,
        })
        .await
        .unwrap();

    let updated = rt
        .update_playbook(
            "playbook-dyn",
            &input("renamed", vec![agent_step("a", &[])]),
            700,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        updated.origin,
        PlaybookOrigin::Dynamic { ref situation_digest, ref plan_id }
            if situation_digest == "dig" && plan_id.as_deref() == Some("brain-plan-1")
    ));
    let record = rt.get_playbook("playbook-dyn").await.unwrap().unwrap();
    assert_eq!(record.origin, "dynamic");
    assert_eq!(record.situation_digest.as_deref(), Some("dig"));
    assert_eq!(record.created_at, 500);
}

#[tokio::test]
async fn latest_dynamic_playbook_probe_over_the_store() {
    let (rt, store) = setup().await;
    rt.create_playbook(&input("fixed", vec![agent_step("a", &[])]), 1_000)
        .await
        .unwrap();
    // Only fixed playbooks: the plan-cache probe finds nothing.
    assert!(store
        .latest_brain_playbook_for("dig")
        .await
        .unwrap()
        .is_none());

    for (created_at, name) in [(5_000, "old"), (6_000, "new")] {
        let s = spec_with_steps(name, vec![agent_step("a", &[])]);
        store
            .save_brain_playbook(&BrainPlaybookRecord {
                id: format!("playbook-{name}"),
                name: name.into(),
                origin: "dynamic".into(),
                situation_digest: Some("dig".into()),
                spec_json: serde_json::to_string(&s).unwrap(),
                created_at,
                updated_at: created_at,
            })
            .await
            .unwrap();
    }
    let latest = store
        .latest_brain_playbook_for("dig")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.name, "new", "newest of the two by created_at");
    assert!(store
        .latest_brain_playbook_for("other")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn create_playbook_with_invalid_input_joins_every_error() {
    let (rt, _store) = setup().await;
    let bad = input(
        "   ",
        vec![agent_step("A b", &["zz"]), agent_step("A b", &[])],
    );
    let err = rt
        .create_playbook(&bad, 1_000)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("name must not be empty"), "{err}");
    assert!(err.contains("not a valid slug"), "{err}");
    assert!(err.contains("unknown step"), "{err}");
    assert!(err.contains("duplicate step name"), "{err}");
    assert!(err.contains("; "), "messages are joined: {err}");
    assert!(
        rt.list_playbooks().await.unwrap().is_empty(),
        "nothing persisted"
    );
}
