use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::ExecutionRef;
use opencoder_dag::{DagSpec, StepKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) async fn load(worker: &Worker, execution: &ExecutionRef) -> Result<Value> {
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&execution.id)
        .cloned()
        .context("DAG execution record missing")?;
    let definition = record
        .assignment
        .definition
        .as_ref()
        .context("DAG definition missing")?;
    let spec = opencoder_dag::decode_spec(definition.get("spec").unwrap_or(definition))
        .map_err(anyhow::Error::msg)?;
    let root = crate::layout::dag::accepted_parent(&record)?.join(&execution.id);
    let resources = opencoder_dag_runtime::resources::frozen_resources(&root)?;
    let mut context = project(&execution.id, &spec, resources.as_ref())?;
    if resources.is_none()
        && record.assignment.index.status != opencoder_core::fleet::ExecutionStatus::Pending
    {
        context["state"] = json!("unavailable");
    }
    Ok(context)
}

fn project(
    run_id: &str,
    spec: &DagSpec,
    resources: Option<&BTreeMap<String, Value>>,
) -> Result<Value> {
    let mut steps = Vec::with_capacity(spec.steps.len());
    if let Some(resources) = resources {
        ensure!(
            resources.len() == spec.steps.len(),
            "frozen resource inventory changed"
        );
    }
    for step in &spec.steps {
        let mut resource = resources
            .map(|pins| {
                pins.get(&step.name)
                    .cloned()
                    .context("frozen resource pin missing")
            })
            .transpose()?;
        if let Some(resource) = resource.as_mut() {
            let digest = resource["sha256"]
                .as_str()
                .context("frozen resource digest missing")?;
            ensure!(
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid frozen resource digest"
            );
            match step.kind.executable() {
                StepKind::Binary {
                    resource: expected, ..
                } => {
                    ensure!(
                        resource["type"] == "binary" && resource["resource"] == *expected,
                        "frozen binary identity changed"
                    );
                    let version = resource["version"]
                        .as_u64()
                        .context("frozen binary version missing")?;
                    let (_, explicit) = opencoder_dag_binary::parse_resource_token(expected)
                        .context("invalid frozen resource token")?;
                    ensure!(
                        version > 0
                            && version <= u32::MAX as u64
                            && explicit.is_none_or(|expected| version == u64::from(expected)),
                        "invalid frozen binary version"
                    );
                }
                StepKind::Agent { agent, .. } => {
                    ensure!(
                        resource["type"] == "agent"
                            && resource["name"] == agent.as_deref().unwrap_or("act"),
                        "frozen Agent identity changed"
                    );
                    resource
                        .as_object_mut()
                        .unwrap()
                        .remove("dependencies_sha256");
                }
                StepKind::Dynamic { .. } => unreachable!(),
            }
        }
        steps.push(
            json!({"name":step.name,"cwd":format!("/workspace/{}",step.name),
            "dynamic":matches!(step.kind,StepKind::Dynamic{..}),"resource":resource}),
        );
    }
    Ok(
        json!({"state":if resources.is_some(){"ready"}else{"preparing"},
        "container_id":format!("dag-run-{run_id}"),"workspace":"/workspace","steps":steps}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> DagSpec {
        opencoder_dag::decode_spec(&json!({"name":"context","steps":[
            {"name":"build","kind":{"type":"binary","resource":"tool"}},
            {"name":"review","kind":{"type":"agent","prompt":"review"}}
        ]}))
        .unwrap()
    }

    #[test]
    fn projects_saved_versions_without_host_paths_or_current_pool_reads() {
        let resources = BTreeMap::from([
            (
                "build".into(),
                json!({"type":"binary","resource":"tool","version":3,"sha256":"a".repeat(64)}),
            ),
            (
                "review".into(),
                json!({"type":"agent","name":"act","sha256":"b".repeat(64),"dependencies_sha256":"private"}),
            ),
        ]);
        let context = project("run-a", &spec(), Some(&resources)).unwrap();
        assert_eq!(context["state"], "ready");
        assert_eq!(context["container_id"], "dag-run-run-a");
        assert_eq!(context["steps"][0]["cwd"], "/workspace/build");
        assert_eq!(context["steps"][0]["resource"]["version"], 3);
        assert!(context["steps"][1]["resource"]
            .get("dependencies_sha256")
            .is_none());
    }

    #[test]
    fn preparation_does_not_invent_resource_versions() {
        let context = project("run-a", &spec(), None).unwrap();
        assert_eq!(context["state"], "preparing");
        assert!(context["steps"][0]["resource"].is_null());
    }

    #[test]
    fn damaged_or_mismatched_snapshots_are_rejected() {
        assert!(project("run-a", &spec(), Some(&BTreeMap::new())).is_err());
        let resources = BTreeMap::from([
            (
                "build".into(),
                json!({"type":"binary","resource":"other","version":3,"sha256":"a".repeat(64)}),
            ),
            (
                "review".into(),
                json!({"type":"agent","name":"act","sha256":"b".repeat(64)}),
            ),
        ]);
        assert!(project("run-a", &spec(), Some(&resources)).is_err());
    }
}
