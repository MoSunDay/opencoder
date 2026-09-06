//! Bounded collectors for output produced by isolated tools.

use anyhow::Result;
use std::io::Read;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) const STREAM_OUTPUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const STRUCTURED_JSON_LIMIT_BYTES: usize = 2 * 1024 * 1024;

/// Read a complete stream without ever retaining more than `limit` bytes.
/// EOF at exactly the limit is accepted; the next byte is a stable error.
pub(crate) async fn read_bounded<R>(
    mut reader: R,
    label: &'static str,
    limit: usize,
) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0u8; 8192];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            return Ok(output);
        }
        let remaining = limit - output.len();
        if count > remaining {
            anyhow::bail!("output_limit_exceeded: {label} exceeds {limit} bytes");
        }
        output.extend_from_slice(&chunk[..count]);
    }
}

/// Read a regular output file with the same exact-boundary contract.
pub(crate) fn read_file_bounded(path: &Path, label: &'static str, limit: usize) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        anyhow::bail!("output_limit_exceeded: {label} exceeds {limit} bytes");
    }
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64 + 1).read_to_end(&mut output)?;
    if output.len() > limit {
        anyhow::bail!("output_limit_exceeded: {label} exceeds {limit} bytes");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exact_limit_is_accepted() {
        let input = [b'x'; 32];
        let output = read_bounded(&input[..], "test output", 32).await.unwrap();
        assert_eq!(output.as_slice(), input);
    }

    #[tokio::test]
    async fn one_byte_over_limit_is_rejected() {
        let input = [b'x'; 33];
        let error = read_bounded(&input[..], "test output", 32)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "output_limit_exceeded: test output exceeds 32 bytes"
        );
    }

    #[test]
    fn file_limit_is_inclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.json");
        std::fs::write(&path, vec![b'x'; 32]).unwrap();
        assert_eq!(read_file_bounded(&path, "test file", 32).unwrap().len(), 32);

        std::fs::write(&path, vec![b'x'; 33]).unwrap();
        let error = read_file_bounded(&path, "test file", 32).unwrap_err();
        assert_eq!(
            error.to_string(),
            "output_limit_exceeded: test file exceeds 32 bytes"
        );
    }
}
