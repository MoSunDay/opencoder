//! Pure dynamic-node input validation, expansion and progress aggregation.
use crate::{StepKind, StepOutcome, StepOutputs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_INSTANCES: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicSource {
    Input { pointer: String },
    StepOutput { step: String, pointer: String },
}

pub fn validate_source(source: &DynamicSource, depends_on: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let pointer = match source {
        DynamicSource::Input { pointer } => pointer,
        DynamicSource::StepOutput { step, pointer } => {
            if !depends_on.contains(step) {
                errors.push(format!(
                    "dynamic source step {step:?} must be in depends_on"
                ));
            }
            pointer
        }
    };
    if !pointer.is_empty() && !pointer.starts_with('/') {
        errors.push("source pointer must be a JSON pointer (empty or starting with /)".into());
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            errors.push("source pointer has an invalid ~ escape".into());
            break;
        }
    }
    errors
}

/// Validate the entire batch before returning any instance inputs.
pub fn validate_items(template: &StepKind, value: &Value) -> Result<Vec<Value>, String> {
    let items = value
        .as_array()
        .ok_or("dynamic source must resolve to an array")?;
    if items.len() > MAX_INSTANCES {
        return Err(format!(
            "dynamic source exceeds {MAX_INSTANCES} instances (got {})",
            items.len()
        ));
    }
    for (index, item) in items.iter().enumerate() {
        match template {
            StepKind::Agent { .. } => {
                item.as_str()
                    .ok_or_else(|| format!("instance {index}: agent input must be a string"))?;
            }
            StepKind::Wasm { .. } => {
                let argv = item.as_array().ok_or_else(|| {
                    format!("instance {index}: wasm input must be an array of strings")
                })?;
                if !argv
                    .iter()
                    .all(|v| v.as_str().is_some_and(|s| !s.contains('\0')))
                {
                    return Err(format!(
                        "instance {index}: argv must contain strings without NUL"
                    ));
                }
            }
            StepKind::Dynamic { .. } => return Err("dynamic template must be agent or wasm".into()),
        }
    }
    Ok(items.clone())
}

pub fn expand(
    source: &DynamicSource,
    template: &StepKind,
    input: &Value,
    outputs: &StepOutputs,
) -> Result<Vec<Value>, String> {
    let (root, pointer) = match source {
        DynamicSource::Input { pointer } => (input, pointer),
        DynamicSource::StepOutput { step, pointer } => (
            outputs
                .get(step)
                .ok_or_else(|| format!("dynamic source output {step:?} is unavailable"))?,
            pointer,
        ),
    };
    let value = root
        .pointer(pointer)
        .ok_or_else(|| format!("dynamic source path {pointer:?} is missing"))?;
    validate_items(template, value)
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub total: usize,
    pub done: usize,
    pub error: usize,
    pub cancelled: usize,
    pub running: usize,
    pub pending: usize,
}

pub fn progress(outcomes: &[Option<StepOutcome>], running: usize) -> Progress {
    let mut p = Progress {
        total: outcomes.len(),
        running,
        ..Default::default()
    };
    for outcome in outcomes {
        match outcome {
            Some(StepOutcome::Done) => p.done += 1,
            Some(StepOutcome::Error) => p.error += 1,
            Some(StepOutcome::Cancelled) => p.cancelled += 1,
            None => p.pending += 1,
        }
    }
    p.pending = p.pending.saturating_sub(running);
    p
}

/// A sibling cancellation belongs to the failed group, not to user cancellation.
pub fn group_outcome(outcomes: &[Option<StepOutcome>]) -> Option<StepOutcome> {
    if outcomes.iter().any(Option::is_none) {
        return None;
    }
    if outcomes.contains(&Some(StepOutcome::Error)) {
        Some(StepOutcome::Error)
    } else if outcomes.contains(&Some(StepOutcome::Cancelled)) {
        Some(StepOutcome::Cancelled)
    } else {
        Some(StepOutcome::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn agent() -> StepKind {
        serde_json::from_value(json!({"type":"agent","prompt":"common"})).unwrap()
    }
    fn wasm() -> StepKind {
        serde_json::from_value(json!({"type":"wasm","command":"t.wasm --format json"})).unwrap()
    }
    #[test]
    fn sources_types_and_empty_batch() {
        let source = DynamicSource::Input {
            pointer: "/items".into(),
        };
        assert_eq!(
            expand(
                &source,
                &agent(),
                &json!({"items":["a", "b"]}),
                &StepOutputs::new()
            )
            .unwrap(),
            vec![json!("a"), json!("b")]
        );
        assert!(expand(&source, &agent(), &json!({}), &StepOutputs::new())
            .unwrap_err()
            .contains("missing"));
        assert!(validate_items(&agent(), &json!(["a", 2])).is_err());
        assert!(validate_items(&wasm(), &json!([["--title", "hello world"], []])).is_ok());
        assert!(validate_items(&wasm(), &json!([["ok"], [1]])).is_err());
        assert!(validate_items(&wasm(), &json!([])).unwrap().is_empty());
        assert!(validate_items(&agent(), &json!(vec!["x"; 1000])).is_ok());
        assert!(validate_items(&agent(), &json!(vec!["x"; 1001])).is_err());
        let source = DynamicSource::StepOutput {
            step: "discover".into(),
            pointer: "/a~1b/~0".into(),
        };
        let outputs = [("discover".into(), json!({"a/b":{"~":["x"]}}))].into();
        assert_eq!(
            expand(&source, &agent(), &Value::Null, &outputs).unwrap(),
            vec![json!("x")]
        );
        assert!(!validate_source(&source, &[]).is_empty());
        assert!(validate_source(&source, &["discover".into()]).is_empty());
    }
    #[test]
    fn failure_cancellation_and_order_independent_counts() {
        let states = [
            Some(StepOutcome::Done),
            Some(StepOutcome::Error),
            Some(StepOutcome::Cancelled),
        ];
        assert_eq!(group_outcome(&states), Some(StepOutcome::Error));
        assert_eq!(group_outcome(&[]), Some(StepOutcome::Done));
        assert_eq!(group_outcome(&[None]), None);
        assert_eq!(
            progress(&[None, None, Some(StepOutcome::Done)], 1),
            Progress {
                total: 3,
                done: 1,
                running: 1,
                pending: 1,
                ..Default::default()
            }
        );
    }
}
