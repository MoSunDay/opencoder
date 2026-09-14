use super::*;
use std::path::Path;

pub async fn all(node: &opencoder_worker::Worker, root: &Path) {
    for kind in [
        ExecutionKind::Project,
        ExecutionKind::Team,
        ExecutionKind::Dag,
        ExecutionKind::Todos,
    ] {
        let id = format!(
            "{}-cancel-matrix",
            serde_json::to_value(kind).unwrap().as_str().unwrap()
        );
        let mut snapshot = project_snapshot("matrix-todo");
        snapshot["todo"]["draft"] = json!("MATRIX_HANG");
        let spec = match kind {
            ExecutionKind::Project => snapshot,
            ExecutionKind::Team => {
                json!({"name":"cancel-team","captain":"act","members":[{"agent":"act"}]})
            }
            ExecutionKind::Dag => {
                json!({"name":"cancel-dag","steps":[{"name":"wait","kind":{"type":"agent","prompt":"MATRIX_HANG"}}]})
            }
            ExecutionKind::Todos => {
                json!({"schema_version":1,"id":"wf-cancel","name":"cancel","objective":"finish","constraints":[],"todos":[{"id":"t1","title":"step","requirement_background":"test","instructions":"MATRIX_HANG MATRIX_CANDIDATE","depends_on":[],"agent":"act","max_attempts":2,"acceptance":{"criteria":"done"}}]})
            }
            _ => unreachable!(),
        };
        let count = fixture::captures(root).len();
        let mut a = assignment(
            node,
            &id,
            kind,
            json!({"prompt":"MATRIX_HANG","action":"plan","run_id":"prun-cancel-matrix"}),
            Some(spec),
        );
        if kind == ExecutionKind::Project {
            a.request.target = Some("matrix-todo".into());
        }
        let reply = node.handle(NodeOperation::Create { assignment: a }).await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let pid = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(record) = fixture::captures(root)
                    .get(count..)
                    .unwrap_or_default()
                    .iter()
                    .find(|r| r["prompt"].as_str().unwrap().contains("MATRIX_HANG"))
                {
                    break record["pid"].as_u64().unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let reply = node
            .handle(NodeOperation::Command {
                execution: ExecutionRef {
                    id: id.clone(),
                    kind,
                },
                command: ExecutionCommand {
                    action: "cancel".into(),
                    input: json!({}),
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let result = settled(node, &id).await;
        assert!(
            ["cancelled", "interrupted"].contains(&result["execution"]["status"].as_str().unwrap()),
            "{result}"
        );
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "Codex process survived {kind:?} cancellation"
        );
    }
}
