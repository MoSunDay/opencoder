use opencoder_core::ontology::Capability;
use serde_json::Value;

pub(super) fn extract(
    item: &Value,
    native: &[Value],
    entry: &mut Capability,
    resources: &mut Vec<Capability>,
) {
    let definition = &item["definition"];
    let kind = item["kind"].as_str().unwrap_or("");
    match kind {
        "brain" => {
            for node in array(&definition["plan"]["nodes"]) {
                if let Some(id) = node["capability_id"].as_str() {
                    entry.dependencies.push(format!("native:{id}"));
                }
            }
        }
        "team" => {
            let team = definition.get("team").unwrap_or(definition);
            for member in array(&team["members"]) {
                if let Some(agent) = member["agent"].as_str() {
                    agent_reference(agent, native, entry);
                }
            }
        }
        "dag" => {
            let spec = definition.get("spec").unwrap_or(definition);
            for step in array(&spec["steps"]) {
                step_references(&step["kind"], native, entry, resources);
            }
        }
        "todos" => {
            for todo in array(&definition["todos"]) {
                if let Some(agent) = todo["agent"].as_str() {
                    agent_reference(agent, native, entry);
                }
            }
            for tool in array(&definition["metadata"]["env_tools"]) {
                if let Some(name) = tool.as_str().or_else(|| tool["reference"].as_str()) {
                    resource_reference("tool", name, entry, resources);
                }
            }
        }
        _ => {}
    }
    entry.dependencies.sort();
    entry.dependencies.dedup();
}

fn array(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn agent_reference(name: &str, native: &[Value], entry: &mut Capability) {
    let matches: Vec<_> = native
        .iter()
        .filter(|value| value["kind"] == "agent" && value["target"] == name)
        .collect();
    if matches.is_empty() {
        entry.dependencies.push(format!("native:agent-{name}"));
    } else {
        for candidate in matches {
            if let Some(id) = candidate["id"].as_str() {
                entry.dependencies.push(format!("native:{id}"));
            }
        }
    }
}

fn step_references(
    kind: &Value,
    native: &[Value],
    entry: &mut Capability,
    resources: &mut Vec<Capability>,
) {
    match kind["type"].as_str() {
        Some("dynamic") => step_references(&kind["template"], native, entry, resources),
        Some("agent") => agent_reference(kind["agent"].as_str().unwrap_or("act"), native, entry),
        Some("binary") => {
            if let Some(reference) = kind["resource"].as_str() {
                resource_reference("binary", reference, entry, resources);
            }
        }
        _ => {}
    }
}

fn resource_reference(
    kind: &str,
    reference: &str,
    entry: &mut Capability,
    resources: &mut Vec<Capability>,
) {
    let key = format!("resource:{kind}:{reference}");
    entry.dependencies.push(key.clone());
    if resources.iter().any(|resource| resource.key == key) {
        return;
    }
    resources.push(Capability {
        key,
        source: "resource-reference".into(),
        source_id: reference.into(),
        kind: "resource".into(),
        name: reference.into(),
        description: "定义中明确声明的资源引用；实际版本与可用性由原模块校验".into(),
        version: reference.into(),
        cli: vec!["opencoder-cli raw call GET PATH".into()],
        api: vec![if kind == "binary" {
            "GET /api/dag/binaries".into()
        } else {
            "GET /api/todo/tools".into()
        }],
        ..Default::default()
    });
}
