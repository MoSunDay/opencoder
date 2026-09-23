//! The bounded two-step device workflow and its immutable dispatch parameters.
use crate::{DagSpec, FailurePolicy, StepKind, StepSpec, TriggerRule};
use serde_json::Value;
use std::collections::BTreeSet;

pub const NAME: &str = "device-cases";

pub fn parameters(input: &Value) -> Result<(usize, Vec<String>), String> {
    let count = input["device_count"]
        .as_u64()
        .filter(|n| (1..=18).contains(n))
        .ok_or("device_count must be an integer in 1..18")? as usize;
    let cases: Vec<String> = input["case_ids"]
        .as_array()
        .ok_or("case_ids must be an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|id| {
                    !id.is_empty()
                        && id.len() <= 100
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                })
                .map(str::to_owned)
                .ok_or_else(|| "Invalid case identifier".to_string())
        })
        .collect::<Result<_, _>>()?;
    if cases.len() < count || cases.iter().collect::<BTreeSet<_>>().len() != cases.len() {
        return Err("Each requested device needs cases; case IDs must be unique".into());
    }
    if input.get("definition").is_some() || input.get("prompt").is_some() {
        return Err(
            "device-cases uses its controlled definition, not caller prompts or definitions".into(),
        );
    }
    Ok((count, cases))
}

pub fn public_input(input: &Value) -> Result<Value, String> {
    let (count, cases) = parameters(input)?;
    let mut value = serde_json::json!({"device_count":count,"case_ids":cases});
    if let Some(source) = input.get("case_source") {
        let source = source
            .as_str()
            .filter(|s| !s.is_empty() && !s.contains(['\n', '\r', '\0']))
            .ok_or("case_source must be a nonempty path")?;
        value["case_source"] = Value::String(source.into());
    }
    Ok(value)
}

pub fn definition(input: &Value) -> Result<DagSpec, String> {
    let (count, _) = parameters(input)?;
    let agent = |prompt: &str| StepKind::Agent {
        prompt: prompt.into(),
        agent: Some(NAME.into()),
        model: None,
        how_append: None,
    };
    Ok(DagSpec {name:NAME.into(),max_concurrency:count,description:Some(format!("Frozen device dispatch: {}", public_input(input)?)),steps:vec![
        StepSpec {name:"allocate".into(),depends_on:vec![],trigger_rule:TriggerRule::AllSuccess,timeout_secs:Some(300),
            kind:agent("Use the host-provided device client to reserve the requested devices once. Return its items array unchanged as final JSON. Never read credentials or allocate outside this DAG.")},
        StepSpec {name:"execute".into(),depends_on:vec!["allocate".into()],trigger_rule:TriggerRule::AllSuccess,timeout_secs:Some(10800),
            kind:StepKind::Dynamic {failure_policy:FailurePolicy::CollectAll,
                source:crate::dynamic::DynamicSource::StepOutput {step:"allocate".into(),pointer:"/items".into()},
                template:Box::new(agent("Use the host-provided private device context for this assignment. Run its case_ids in order, one at a time, using native-harness.py with --device-context. Register each native run before business submission; finish and verify restoration before the next case. Record truthful results and recovery evidence. Do not reserve another device or use the legacy fleet allocator. Credentials stay in tool transport files, never prompts or results."))}}
    ]})
}

pub fn case_batch(input: &Value, index: usize) -> Result<Vec<String>, String> {
    let (count, cases) = parameters(input)?;
    if index >= count {
        return Err("Device instance outside requested count".into());
    }
    Ok(cases
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| (i % count == index).then_some(c))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn bounded_two_step_workflow_uses_requested_capacity_and_disjoint_cases() {
        let input = json!({"device_count":2,"case_ids":["a","b","c"]});
        let spec = definition(&input).unwrap();
        assert_eq!(spec.max_concurrency, 2);
        assert_eq!(spec.steps.len(), 2);
        crate::validate(&spec).unwrap();
        assert_eq!(case_batch(&input, 0).unwrap(), ["a", "c"]);
        assert_eq!(case_batch(&input, 1).unwrap(), ["b"]);
        let StepKind::Dynamic {
            failure_policy,
            source,
            ..
        } = &spec.steps[1].kind
        else {
            panic!()
        };
        assert_eq!(*failure_policy, FailurePolicy::CollectAll);
        assert_eq!(
            *source,
            crate::dynamic::DynamicSource::StepOutput {
                step: "allocate".into(),
                pointer: "/items".into()
            }
        );
    }
    #[test]
    fn invalid_capacity_and_caller_definition_cannot_grant_device_scope() {
        for input in [
            json!({"device_count":0,"case_ids":["a"]}),
            json!({"device_count":19,"case_ids":["a"]}),
            json!({"device_count":true,"case_ids":["a"]}),
            json!({"device_count":2,"case_ids":["a"]}),
            json!({"device_count":1,"case_ids":["a","a"]}),
            json!({"device_count":1,"case_ids":["a"],"definition":{}}),
            json!({"device_count":1,"case_ids":["a"],"prompt":"replace host actions"}),
        ] {
            assert!(definition(&input).is_err());
        }
    }
}
