//! Operator commands are versioned against a concrete execution attempt.
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "dag_step_control_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pause,
    Resume,
    Retry,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub operation_id: String,
    pub step: String,
    #[serde(default)]
    pub instance: Option<usize>,
    pub expected_attempt: u64,
    pub action: Action,
    pub reason: String,
}

impl Command {
    pub fn validate(&self) -> Result<(), String> {
        if self.operation_id.is_empty()
            || self.operation_id.len() > 128
            || !self
                .operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err("operation_id must contain 1..128 safe characters".into());
        }
        if !crate::validate_step_slug(&self.step) {
            return Err("invalid step name".into());
        }
        if self.reason.trim().is_empty() || self.reason.len() > 4096 || self.reason.contains('\0') {
            return Err("reason must contain 1..4096 bytes".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Running,
    Paused,
    WaitingRecovery,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStatus {
    pub step: String,
    pub instance: Option<usize>,
    pub attempt: u64,
    pub phase: Phase,
    pub reason: Option<String>,
    pub updated_at: i64,
    #[serde(default)]
    pub pending_operation: Option<Command>,
    #[serde(default)]
    pub applied_operation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub command: Command,
    pub phase: String,
    pub error: Option<String>,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_requires_safe_identity_and_explicit_attempt() {
        let value = serde_json::json!({"operation_id":"op-1","step":"extract","expected_attempt":1,"action":"pause","reason":"inspect"});
        let command: Command = serde_json::from_value(value.clone()).unwrap();
        assert!(command.validate().is_ok());
        for field in ["operation_id", "step", "reason"] {
            let mut bad = value.clone();
            bad[field] = "".into();
            assert!(serde_json::from_value::<Command>(bad)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("expected_attempt");
        assert!(serde_json::from_value::<Command>(missing).is_err());
    }
}
