use crate::AppState;
use anyhow::{Context, Result};
use opencoder_core::ontology::{platform_capabilities, Capability};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn snapshot(state: &Arc<AppState>) -> Result<Vec<Capability>> {
    let native = crate::api::brain_runs::catalog::definition_snapshot(state).await?;
    project(&native)
}

fn project(native: &[Value]) -> Result<Vec<Capability>> {
    let mut entries = platform_capabilities();
    for item in native {
        let id = item["id"]
            .as_str()
            .context("capability definition missing id")?;
        let kind = item["kind"]
            .as_str()
            .context("capability definition missing kind")?;
        let target = item["target"].as_str().unwrap_or(id);
        let mut entry = Capability {
            key: format!("native:{id}"),
            source: "native".into(),
            source_id: id.into(),
            kind: kind.into(),
            name: target.into(),
            description: text(item, "summary"),
            version: format!(
                "{}:{}",
                text(item, "version"),
                fingerprint(&item["definition"])
            ),
            input_desc: text(item, "input_desc"),
            output_desc: text(item, "output_desc"),
            required_inputs: strings(&item["required_inputs"]),
            required_outputs: strings(&item["required_outputs"]),
            cli: vec![format!("opencoder-cli {} --help", domain(kind))],
            api: entrypoints(kind, target, id),
            dependencies: vec![format!("platform:{}", domain(kind))],
            ..Default::default()
        };
        if !item["unavailable_reason"].is_null() {
            entry.issues.push(
                item["unavailable_reason"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| item["unavailable_reason"].to_string()),
            );
        }
        super::dependencies::extract(item, native, &mut entry, &mut entries);
        entries.push(entry);
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries.dedup_by(|a, b| a.key == b.key && a == b);
    Ok(entries)
}

fn fingerprint(value: &Value) -> String {
    // Reuse the platform's stable SHA-256 primitive; no definition bodies or credentials
    // are copied into the public Ontology description.
    opencoder_core::identity::token_hash(&value.to_string())
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").into()
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn domain(kind: &str) -> &'static str {
    match kind {
        "agent" | "operator" => "agents",
        "team" => "teams",
        "dag" => "dag",
        "todos" => "todo",
        _ => "brain",
    }
}
fn encoded(value: &str) -> String {
    // Percent-encode a path segment, including '/' in native targets.
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
fn entrypoints(kind: &str, target: &str, id: &str) -> Vec<String> {
    let source = match kind {
        "agent" | "operator" => format!("GET /api/agents/{}/meta", encoded(target)),
        "team" => "GET /api/teams".into(),
        "dag" => "GET /api/dag/defs".into(),
        "todos" => {
            let (name, version) = target.split_once('/').unwrap_or((target, ""));
            format!(
                "GET /api/todo/templates/{}/{}/context.json",
                encoded(name),
                encoded(version)
            )
        }
        "brain" => {
            let (id, version) = target.rsplit_once('@').unwrap_or((target, ""));
            format!(
                "GET /api/brain/plan-defs/{}/versions/{}",
                encoded(id),
                encoded(version)
            )
        }
        _ => format!("GET /api/brain/capabilities/{}", encoded(id)),
    };
    vec![
        source,
        "GET /api/brain/library".into(),
        "POST /api/executions".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn six_native_kinds_keep_explicit_references_without_copying_private_configuration() {
        let mut values = vec![
            json!({"id":"a","kind":"agent","target":"act","definition":{"prompt":"original","token":"private-test-value"}}),
            json!({"id":"o","kind":"operator","target":"act","definition":{}}),
            json!({"id":"t","kind":"team","target":"team","definition":{"members":[{"agent":"act"}]}}),
            json!({"id":"d","kind":"dag","target":"dag","definition":{"spec":{"steps":[{"kind":{"type":"dynamic","template":{"type":"binary","resource":"tool@v1"}}},{"kind":{"type":"agent","agent":"absent"}}]}}}),
            json!({"id":"w","kind":"todos","target":"template/v1","definition":{"todos":[{"agent":"act"}],"metadata":{"env_tools":["tool/ref"],"env_vars":{"SECRET":"private-test-value"}}}}),
            json!({"id":"b","kind":"brain","target":"plan@1","definition":{"plan":{"nodes":[{"capability_id":"a"}]}}}),
        ];
        let entries = project(&values).unwrap();
        let by = |key: &str| entries.iter().find(|entry| entry.key == key).unwrap();
        assert!(by("native:t").dependencies.contains(&"native:a".into()));
        assert!(by("native:d")
            .dependencies
            .contains(&"resource:binary:tool@v1".into()));
        assert!(by("native:d")
            .dependencies
            .contains(&"native:agent-absent".into()));
        assert!(by("native:w")
            .dependencies
            .contains(&"resource:tool:tool/ref".into()));
        assert!(by("native:b").dependencies.contains(&"native:a".into()));
        assert!(!serde_json::to_string(&entries)
            .unwrap()
            .contains("private-test-value"));
        values[0]["definition"]["prompt"] = json!("changed");
        let updated = project(&values).unwrap();
        assert_ne!(
            by("native:a").version,
            updated
                .iter()
                .find(|entry| entry.key == "native:a")
                .unwrap()
                .version
        );
        assert!(project(&[json!({"kind":"agent"})]).is_err());
    }
}
