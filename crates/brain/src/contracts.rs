//! Pure validation of explicit capability fields and bounded decision evidence.
use anyhow::{ensure, Result};
use opencoder_core::brain::BRAIN_EVIDENCE_MAX_BYTES;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub fn validate_fields(fields: &[String]) -> Result<()> {
    ensure!(fields.len() <= 64, "at most 64 required fields are allowed");
    let mut seen = BTreeSet::new();
    for field in fields {
        ensure!(
            !field.is_empty() && field.trim() == field && field.len() <= 128,
            "required field names must contain 1..128 bytes without surrounding whitespace"
        );
        ensure!(seen.insert(field), "duplicate required field {field}");
    }
    Ok(())
}

fn present(value: &Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|text| !text.trim().is_empty())
}

pub fn validate_inputs(required: &[String], inputs: &Map<String, Value>) -> Result<()> {
    validate_fields(required)?;
    for field in required {
        ensure!(
            inputs.get(field).is_some_and(present),
            "required capability input {field} is missing, null or blank"
        );
    }
    Ok(())
}

pub fn validate_output(required: &[String], output: &Value) -> Result<()> {
    validate_fields(required)?;
    ensure!(present(output), "capability output is null or blank");
    for field in required {
        ensure!(
            output.get(field).is_some_and(present),
            "required capability output field {field} is missing, null or blank"
        );
    }
    ensure!(
        serde_json::to_vec(output)?.len() <= BRAIN_EVIDENCE_MAX_BYTES,
        "capability decision evidence exceeds {BRAIN_EVIDENCE_MAX_BYTES} bytes; return concise structured evidence and artifact references"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_fields_preserve_false_and_zero_but_reject_missing_values() {
        let fields = vec!["passed".into(), "count".into()];
        let output = json!({"passed":false,"count":0});
        validate_inputs(&fields, output.as_object().unwrap()).unwrap();
        validate_output(&fields, &output).unwrap();
        for invalid in [json!({}), json!({"passed":null}), json!({"passed":" "})] {
            assert!(validate_inputs(&fields, invalid.as_object().unwrap()).is_err());
            assert!(validate_output(&fields, &invalid).is_err());
        }
        for invalid in [json!(null), json!(" ")] {
            assert!(validate_output(&[], &invalid).is_err());
        }
    }

    #[test]
    fn ambiguous_field_lists_and_oversized_evidence_are_rejected() {
        for fields in [
            vec!["".into()],
            vec![" x".into()],
            vec!["x".into(); 2],
            vec!["x".repeat(129)],
        ] {
            assert!(validate_fields(&fields).is_err());
        }
        assert!(validate_fields(&(0..65).map(|n| format!("f{n}")).collect::<Vec<_>>()).is_err());
        validate_output(&[], &json!("evidence with an artifact reference")).unwrap();
        assert!(validate_output(&[], &json!("x".repeat(BRAIN_EVIDENCE_MAX_BYTES))).is_err());
    }
}
