use crate::fleet::ExecutionRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub execution: ExecutionRef,
    pub step: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
}
