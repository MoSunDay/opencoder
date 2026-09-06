use super::ExecutionIndex;
use serde::{Deserialize, Serialize};

pub const INDEX_REPORT_BATCH_SIZE: usize = 256;

/// One ordered fragment of a node's complete execution-index snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReportEnvelope {
    pub report_id: u64,
    #[serde(flatten)]
    pub part: IndexReportPart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum IndexReportPart {
    Begin,
    Batch { records: Vec<ExecutionIndex> },
    End,
}

impl IndexReportEnvelope {
    pub fn begin(report_id: u64) -> Self {
        Self {
            report_id,
            part: IndexReportPart::Begin,
        }
    }

    pub fn batch(report_id: u64, records: Vec<ExecutionIndex>) -> Self {
        Self {
            report_id,
            part: IndexReportPart::Batch { records },
        }
    }

    pub fn end(report_id: u64) -> Self {
        Self {
            report_id,
            part: IndexReportPart::End,
        }
    }
}
