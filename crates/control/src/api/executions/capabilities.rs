//! Feature negotiation uses the existing Brain action, so old nodes can reject
//! a request before receiving operation variants they cannot deserialize.
use crate::AppState;
use opencoder_core::fleet::*;
use serde_json::{json, Value};

pub(super) const DYNAMIC_DAG: &str = "dag_dynamic_v1";

pub(super) fn requires_dynamic(kind: ExecutionKind, definition: Option<&Value>) -> bool {
    let Some(definition) = definition else {
        return false;
    };
    let spec = match kind {
        ExecutionKind::Dag => definition.get("spec").unwrap_or(definition).clone(),
        ExecutionKind::Project if definition["todo"]["executor_kind"] == "dag" => definition
            ["todo"]["executor_spec"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null),
        _ => return false,
    };
    spec["steps"]
        .as_array()
        .is_some_and(|steps| steps.iter().any(|step| step["kind"]["type"] == "dynamic"))
}

fn supports(reply: &RpcReply) -> bool {
    (200..300).contains(&reply.status)
        && reply.body["compatible"] == true
        && reply.body["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|f| f == DYNAMIC_DAG))
}

pub(super) async fn probe(
    state: &AppState,
    node_id: &str,
    execution: ExecutionRef,
    input: Value,
    dynamic: bool,
) -> Result<(), RpcReply> {
    let reply = state
        .hub
        .call(
            node_id,
            NodeOperation::Brain {
                execution,
                action: "capability_probe".into(),
                input,
            },
        )
        .await;
    if !(200..300).contains(&reply.status) {
        return Err(reply);
    }
    if dynamic && !supports(&reply) {
        return Err(RpcReply::error(
            409,
            format!("node {node_id} does not support {DYNAMIC_DAG}; upgrade the owning node"),
        ));
    }
    Ok(())
}

pub(crate) async fn require_dynamic(
    state: &AppState,
    index: &ExecutionIndex,
) -> Result<(), RpcReply> {
    probe(
        state,
        &index.node_id,
        index.execution_ref(),
        json!({}),
        true,
    )
    .await
}

pub(super) fn operation_requires_dynamic(operation: &NodeOperation) -> bool {
    matches!(
        operation,
        NodeOperation::DagInstances { .. } | NodeOperation::DagInstanceEvents { .. }
    ) || matches!(operation, NodeOperation::Artifact { request } if request.index.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_positive_probes_do_not_advertise_new_protocol_operations() {
        assert!(!supports(&RpcReply::ok(json!({"compatible":true}))));
        assert!(!supports(&RpcReply::ok(
            json!({"compatible":true,"features":[]})
        )));
        assert!(!supports(&RpcReply::ok(
            json!({"compatible":false,"features":[DYNAMIC_DAG]})
        )));
        assert!(supports(&RpcReply::ok(
            json!({"compatible":true,"features":[DYNAMIC_DAG]})
        )));
    }

    #[test]
    fn detects_dynamic_in_catalog_inline_and_project_definitions() {
        let spec = json!({"steps":[{"kind":{"type":"dynamic"}}]});
        assert!(requires_dynamic(ExecutionKind::Dag, Some(&spec)));
        assert!(requires_dynamic(
            ExecutionKind::Dag,
            Some(&json!({"spec":spec}))
        ));
        assert!(requires_dynamic(
            ExecutionKind::Project,
            Some(&json!({"todo":{
                "executor_kind":"dag", "executor_spec":spec.to_string()
            }}))
        ));
        assert!(!requires_dynamic(ExecutionKind::Agent, Some(&spec)));
        assert!(!requires_dynamic(
            ExecutionKind::Dag,
            Some(&json!({"steps":[{"kind":{"type":"agent"}}]}))
        ));
    }
}
