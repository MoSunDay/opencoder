use super::super::config::{Inventory, InventoryUpdate};
use super::*;
use opencoder_core::fleet::inventory::*;

struct Source {
    log: InventoryLog,
    pages: InventoryPages,
    generation: String,
    inventory: Inventory,
    queries: Vec<InventoryQuery>,
}

#[tokio::test]
async fn runtime_pages_flow_into_host_deltas_and_generation_resets() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let host = Host::open(&root.path().join("host"), "node".into(), "test".into(), 2)
        .await
        .unwrap();
    let mut log = InventoryLog::default();
    for n in 0..600 {
        log.set(ExecutionIndex {
            id: format!("agent-{n:04}"),
            created_at: n,
            node_id: host.registration.id.clone(),
            kind: ExecutionKind::Agent,
            status: ExecutionStatus::Done,
        });
    }
    let mut snapshot = host.snapshot();
    snapshot.generation = "generation-a".into();
    snapshot.ready = true;
    let source = Arc::new(tokio::sync::Mutex::new(Source {
        log,
        pages: Default::default(),
        generation: snapshot.generation.clone(),
        queries: vec![],
        inventory: Inventory {
            runtime_id: Some("runtime".into()),
            build: serde_json::to_value(opencoder_core::version::build_info()).unwrap(),
            owned_processes: 0,
            registration: host.registration.clone(),
            snapshot,
            indexes: vec![],
            can_hibernate: true,
        },
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let state = source.clone();
    let app = axum::Router::new().route(
        "/inventory/changes",
        axum::routing::get(
            move |axum::extract::Query(query): axum::extract::Query<InventoryQuery>| {
                let state = state.clone();
                async move {
                    let mut s = state.lock().await;
                    s.queries.push(query.clone());
                    let generation = s.generation.clone();
                    let changes = if let Some(token) = query.page {
                        s.pages.next(&generation, &token).unwrap()
                    } else {
                        let after = if query.generation.as_ref() == Some(&generation) {
                            query.after
                        } else {
                            None
                        };
                        let changes = s.log.read(after);
                        s.pages.begin(&generation, changes)
                    };
                    axum::Json(InventoryUpdate {
                        inventory: s.inventory.clone(),
                        changes,
                    })
                }
            },
        ),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store.register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
        id: "runtime".into(), release_id: "runtime".into(), mode: "staged".into(),
        config: json!({"endpoint":endpoint,"data_dir":root.path().join("runtime"),"unit":"opencoder-runtime-test.service"}),
    }).await.unwrap();
    host.store.activate_runtime("runtime").await.unwrap();
    let full = host.collect_inventory_changes(None).await.unwrap();
    assert_eq!(full.records.len(), 600);
    assert_eq!(source.lock().await.queries.len(), 3);
    let idle = host
        .collect_inventory_changes(Some(full.revision))
        .await
        .unwrap();
    assert!(!idle.full);
    assert!(idle.records.is_empty());
    assert_eq!(source.lock().await.queries.last().unwrap().after, Some(600));
    {
        let mut s = source.lock().await;
        s.log.remove("agent-0001");
        let mut changed = full.records[0].clone();
        changed.status = ExecutionStatus::Running;
        s.log.set(changed);
    }
    let delta = host
        .collect_inventory_changes(Some(idle.revision))
        .await
        .unwrap();
    assert_eq!(delta.records.len(), 1);
    assert_eq!(delta.records[0].status, ExecutionStatus::Running);
    assert_eq!(delta.removed_ids, vec!["agent-0001"]);
    {
        let mut s = source.lock().await;
        s.generation = "generation-b".into();
        s.inventory.snapshot.generation = s.generation.clone();
        s.log.remove("agent-0002");
    }
    let restarted = host
        .collect_inventory_changes(Some(delta.revision))
        .await
        .unwrap();
    assert!(restarted.records.is_empty());
    assert_eq!(restarted.removed_ids, vec!["agent-0002"]);
    assert_eq!(host.sync_inventory().await.unwrap().len(), 598);
    assert!(host.snapshot().ready);
    server.abort();
}
