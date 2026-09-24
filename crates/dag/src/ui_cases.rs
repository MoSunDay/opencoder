//! Immutable, controlled UI case dispatch contract.
use crate::{DagSpec, FailurePolicy, StepKind, StepSpec, TriggerRule};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const NAME: &str = "uicase-regression";

pub fn public_input(input: &Value) -> Result<Value, String> {
    if input.get("definition").is_some()
        || input.get("prompt").is_some()
        || input.get("vm").is_some()
        || input.get("owner").is_some()
    {
        return Err("UI device identity and definition are host controlled".into());
    }
    let count = input["device_count"]
        .as_u64()
        .filter(|n| (1..=18).contains(n))
        .ok_or("device_count must be in 1..18")? as usize;
    let cases = input["cases"].as_array().ok_or("cases must be an array")?;
    if cases.len() < count || cases.len() > 1266 {
        return Err("Every device needs at least one case".into());
    }
    let mut ids = BTreeSet::new();
    let mut frozen = Vec::with_capacity(cases.len());
    for row in cases {
        let id = row["case_id"]
            .as_str()
            .filter(|s| valid_id(s))
            .ok_or("Invalid UI case ID")?;
        let hash = row["spec_sha256"]
            .as_str()
            .filter(|s| hex_hash(s))
            .ok_or("Invalid UI spec hash")?;
        let version = row["input_version"]
            .as_str()
            .filter(|s| valid_id(s))
            .ok_or("Invalid UI input version")?;
        if !ids.insert(id) {
            return Err("Duplicate UI case ID".into());
        }
        frozen.push(
            json!({"case_id":id,"spec_sha256":hash.to_ascii_lowercase(),"input_version":version}),
        );
    }
    let source = input["case_source"]
        .as_str()
        .filter(|s| s.starts_with('/') && !s.contains(['\n', '\r', '\0']))
        .ok_or("case_source must be an absolute path")?;
    Ok(json!({"device_count":count,"cases":frozen,"case_source":source}))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn hex_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn case_batch(input: &Value, index: usize) -> Result<Vec<Value>, String> {
    let frozen = public_input(input)?;
    let count = frozen["device_count"].as_u64().unwrap() as usize;
    if index >= count {
        return Err("UI device instance outside requested count".into());
    }
    Ok(frozen["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter_map(|(i, case)| (i % count == index).then_some(case.clone()))
        .collect())
}

pub fn definition(input: &Value) -> Result<DagSpec, String> {
    let frozen = public_input(input)?;
    let count = frozen["device_count"].as_u64().unwrap() as usize;
    let agent = |prompt: &str| StepKind::Agent {
        prompt: prompt.into(),
        agent: Some(NAME.into()),
        model: None,
        how_append: None,
    };
    Ok(DagSpec {
        name: NAME.into(), max_concurrency: count,
        description: Some(format!("Frozen UI case dispatch: {frozen}")),
        steps: vec![
            StepSpec { name: "allocate".into(), depends_on: vec![],
                trigger_rule: TriggerRule::AllSuccess, timeout_secs: Some(300),
                kind: agent("Use the host-provided private UI device client to reserve the requested devices once. Return its items array unchanged as final JSON.") },
            StepSpec { name: "execute".into(), depends_on: vec!["allocate".into()],
                trigger_rule: TriggerRule::AllSuccess, timeout_secs: Some(10800),
                kind: StepKind::Dynamic { failure_policy: FailurePolicy::CollectAll,
                    source: crate::dynamic::DynamicSource::StepOutput {
                        step: "allocate".into(), pointer: "/items".into(),
                    },
                    template: Box::new(agent("Run only the frozen UI cases assigned by the host, one at a time. Register work before device operations; finish capture and independently verified restoration before the next case. Return case verdict, assertion evidence, capture hashes and restoration receipt. Never use the legacy allocator or caller-supplied device identity.")),
                } },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_cases_are_disjoint_and_caller_cannot_grant_a_device() {
        let input = json!({"device_count":2,"case_source":"/private/specs",
            "cases":[{"case_id":"BITS-1-a","spec_sha256":"a".repeat(64),"input_version":"v1"},
                     {"case_id":"BITS-2-b","spec_sha256":"b".repeat(64),"input_version":"v1"}]});
        crate::validate(&definition(&input).unwrap()).unwrap();
        assert_eq!(case_batch(&input, 0).unwrap()[0]["case_id"], "BITS-1-a");
        assert_eq!(case_batch(&input, 1).unwrap()[0]["case_id"], "BITS-2-b");
        for field in ["vm", "owner", "definition", "prompt"] {
            let mut forged = input.clone();
            forged[field] = json!("caller");
            assert!(definition(&forged).is_err());
        }
    }
}
