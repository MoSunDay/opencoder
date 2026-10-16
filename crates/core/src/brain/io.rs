//! Frozen execution IO contract; absent markers retain the legacy wire format.
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::Value;

pub const EXECUTION_IO_READER_VERSION: u32 = 1;
// A bridge build sets this to 0. Enable 1 only after the reader bridge is the
// recorded rollback baseline; the release preflight enforces that order.
pub const EXECUTION_IO_WRITER_VERSION: u32 = 1;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ExecutionIoCompatibility {
    pub reader: u32,
    pub writer: u32,
}

impl ExecutionIoCompatibility {
    pub fn current() -> Self {
        Self {
            reader: EXECUTION_IO_READER_VERSION,
            writer: EXECUTION_IO_WRITER_VERSION,
        }
    }
}

pub fn execution_io_version(input: &Value) -> Result<u32> {
    let Some(value) = input.get("execution_io_version") else {
        return Ok(0);
    };
    let version = value
        .as_u64()
        .context("execution_io_version must be an integer")?;
    ensure!(
        version <= u64::from(EXECUTION_IO_READER_VERSION),
        "unsupported execution_io_version {version}"
    );
    Ok(version as u32)
}

pub fn is_legacy_io(version: &u32) -> bool {
    *version == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_is_legacy_and_invalid_versions_are_rejected() {
        assert_eq!(execution_io_version(&json!({})).unwrap(), 0);
        assert_eq!(
            execution_io_version(&json!({"execution_io_version":1})).unwrap(),
            1
        );
        for value in [json!(null), json!(-1), json!(1.5), json!("1"), json!(2)] {
            assert!(execution_io_version(&json!({"execution_io_version":value})).is_err());
        }
    }
}
