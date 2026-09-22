//! Private execution inputs are never included in public CreateExecution readback.
use opencoder_core::fleet::{CreateExecution, ExecutionKind, PrivateExecutionContext};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Submission {
    #[serde(flatten)]
    pub request: CreateExecution,
    #[serde(default)]
    pub private_context: Option<PrivateExecutionContext>,
}

pub(super) fn validate(
    request: &CreateExecution,
    private: Option<&PrivateExecutionContext>,
) -> Result<(), &'static str> {
    if let Some(private) = private {
        if request.kind != ExecutionKind::Dag {
            return Err("private task files require a DAG execution");
        }
        private.validate(opencoder_core::message::now_ms())?;
    }
    Ok(())
}

pub(super) fn validate_definition(
    private: Option<&PrivateExecutionContext>,
    definition: Option<&serde_json::Value>,
) -> Result<(), &'static str> {
    let Some(private) = private else {
        return Ok(());
    };
    let definition = definition.ok_or("private DAG definition missing")?;
    let bytes = serde_json::to_string(definition.get("spec").unwrap_or(definition))
        .map_err(|_| "invalid DAG definition")?;
    if opencoder_core::token_hash(&bytes) != private.definition_sha256 {
        return Err("pinned DAG definition changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_submission_fields_do_not_enter_public_request() {
        let submission:Submission=serde_json::from_value(serde_json::json!({"id":"dag-example","kind":"dag","target":"task","input":{"prompt":"work"},"private_context":{"expires_at_ms":1000,"image_digest":format!("sha256:{}","a".repeat(64)),"definition_sha256":"b".repeat(64),"files":{"credential":"fixture-private-token"}}})).unwrap();
        assert!(submission.private_context.is_some());
        let public = serde_json::to_string(&submission.request).unwrap();
        assert!(!public.contains("private_context"));
        assert!(!public.contains("fixture-private-token"));
    }
}
