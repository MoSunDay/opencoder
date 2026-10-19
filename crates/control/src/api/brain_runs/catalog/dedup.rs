//! Presentation merges do not remove identifiers used by saved plans. Execution
//! resolution continues to read the complete catalog and immutable snapshots.
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
#[path = "agents.rs"]
mod agents;

pub(super) async fn select(
    root: Option<PathBuf>,
    mut values: Vec<Value>,
) -> anyhow::Result<(Vec<Value>, Vec<Value>)> {
    read_resources(root, move || {
        prepare(&mut values);
        let selected = collapse(values.clone());
        (values, selected)
    })
    .await
}

async fn read_resources<T: Send + 'static>(
    root: Option<PathBuf>,
    read: impl FnOnce() -> T + Send + 'static,
) -> anyhow::Result<T> {
    // Resource content hashing can involve large files. Keep it off the async
    // workers that also serve admission and health checks, retaining its scope.
    Ok(tokio::task::spawn_blocking(move || {
        opencoder_core::agent::scope::with_root_sync(root, read)
    })
    .await?)
}

fn key(value: &Value) -> Option<String> {
    if !value["definition"].is_object() || value.get("unavailable_reason").is_some() {
        return None;
    }
    let identity = value.get("_catalog_agent_identity").cloned();
    Some(json!({"kind":value["kind"],"target":if identity.is_some(){Value::Null}else{value["target"].clone()},"definition":identity.unwrap_or_else(||value["definition"].clone()),
        "inputs":fields(value,"required_inputs")?,
        "outputs":fields(value,"required_outputs")?}).to_string())
}

fn fields(value: &Value, name: &str) -> Option<Value> {
    let mut fields = match value.get(name) {
        None => Vec::new(),
        Some(Value::Array(fields)) if fields.iter().all(Value::is_string) => fields.clone(),
        _ => return None,
    };
    fields.sort_by_key(Value::to_string);
    fields.dedup();
    Some(json!(fields))
}

fn rank(value: &Value) -> (u8, String) {
    (
        match value["version"].as_str() {
            Some("stored") => 0,
            Some("builtin") => 1,
            _ => 2,
        },
        value["id"].as_str().unwrap_or("").into(),
    )
}

pub(super) fn prepare(values: &mut [Value]) {
    for value in values {
        if let Some(identity) = agents::identity(value) {
            value["_catalog_agent_identity"] = identity;
        }
    }
}

pub(super) fn collapse(mut values: Vec<Value>) -> Vec<Value> {
    values.sort_by_key(rank);
    let mut positions = BTreeMap::new();
    let mut result: Vec<Value> = Vec::new();
    for value in values {
        let Some(key) = key(&value) else {
            result.push(value);
            continue;
        };
        if let Some(&index) = positions.get(&key) {
            let kept: &mut Value = &mut result[index];
            // Retain all alternative descriptions for inspection; prefer authored
            // descriptions in the visible fields, filling only empty values.
            let source = json!({"id":value["id"],"target":value["target"],"summary":value["summary"],"input_desc":value["input_desc"],"output_desc":value["output_desc"]});
            kept.as_object_mut()
                .unwrap()
                .entry("equivalent_sources")
                .or_insert(json!([]))
                .as_array_mut()
                .unwrap()
                .push(source);
            for field in ["summary", "input_desc", "output_desc"] {
                if kept[field].as_str().unwrap_or("").trim().is_empty() {
                    kept[field] = value[field].clone();
                }
            }
        } else {
            positions.insert(key, result.len());
            result.push(value);
        }
    }
    for value in &mut result {
        if let Some(fields) = value.as_object_mut() {
            fields.remove("_catalog_agent_identity");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn slow_resource_reads_yield_to_requests_and_keep_their_root() {
        let (started, pending) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let root = PathBuf::from("/catalog-resource-test");
        let expected = root.clone();
        let task = tokio::spawn(read_resources(Some(root), move || {
            started
                .send(opencoder_core::agent::scope::current_root())
                .unwrap();
            gate.recv_timeout(std::time::Duration::from_secs(5))
        }));
        assert_eq!(pending.await.unwrap(), Some(expected));
        assert!(opencoder_core::agent::scope::current_root().is_none());
        // This task must run before the blocking read can finish. An inline
        // read stalls this single async worker and the gate times out.
        release.send(()).unwrap();
        task.await.unwrap().unwrap().unwrap();
    }

    #[test]
    fn authored_descriptions_win_without_losing_alternative_ids_or_contracts() {
        let stored = json!({"id":"registered","kind":"dag","target":"one","version":"stored","definition":{"spec":{}},"input_desc":"specific inputs","required_inputs":["b","a"]});
        let mut current = stored.clone();
        current["id"] = json!("dag-one");
        current["version"] = json!("current");
        current["input_desc"] = json!("generic inputs");
        current["required_inputs"] = json!(["a", "b"]);
        let mut other = current.clone();
        other["id"] = json!("different-contract");
        other["required_inputs"] = json!([{"name":"file"}]);
        let result = collapse(vec![current, other, stored]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["id"], "registered");
        assert_eq!(result[0]["input_desc"], "specific inputs");
        assert_eq!(result[0]["equivalent_sources"][0]["id"], "dag-one");
    }
    #[test]
    fn unavailable_or_differently_bound_capabilities_are_not_merged() {
        let a = json!({"id":"a","kind":"agent","target":"a","definition":{"prompt":"same"}});
        let mut b = a.clone();
        b["target"] = json!("b");
        b["id"] = json!("b");
        assert_eq!(collapse(vec![a, b]).len(), 2);
        assert_eq!(
            collapse(vec![
                json!({"id":"missing-one"}),
                json!({"id":"missing-two"})
            ])
            .len(),
            2
        );
    }
}
