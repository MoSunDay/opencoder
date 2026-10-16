use crate::fleet::ExecutionRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub execution: ExecutionRef,
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn artifact_references_read_historical_files_and_preserve_zero_instance_index() {
        let old = json!({"execution":{"id":"dag-a","kind":"dag"},"step":"build",
            "file":"report.txt","sha256":"a".repeat(64),"bytes":0});
        let reference: ArtifactRef = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(reference.index, None);
        assert_eq!(serde_json::to_value(reference).unwrap(), old);
        let mut dynamic = old;
        dynamic["index"] = json!(0);
        let reference: ArtifactRef = serde_json::from_value(dynamic.clone()).unwrap();
        assert_eq!(reference.index, Some(0));
        assert_eq!(serde_json::to_value(reference).unwrap(), dynamic);
    }
}
