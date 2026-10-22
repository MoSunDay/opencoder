use serde::{Deserialize, Serialize};

/// The reader bridge uses writer 0; the subsequent writer release uses 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportStateCompatibility {
    pub reader: u32,
    pub writer: u32,
}
impl ReportStateCompatibility {
    pub const fn current() -> Self {
        Self {
            reader: 1,
            writer: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_never_exceeds_reader() {
        let current = ReportStateCompatibility::current();
        assert!(current.writer <= current.reader);
        assert_eq!(
            serde_json::from_str::<ReportStateCompatibility>(
                &serde_json::to_string(&current).unwrap()
            )
            .unwrap(),
            current
        );
    }
}
