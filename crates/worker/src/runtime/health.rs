//! Fail-closed local storage health used for placement and admission.

use anyhow::{Context, Result};
use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path, sync::Arc};

pub const MIN_AVAILABLE_RATIO: f64 = 0.20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StorageCapacity {
    pub available_blocks: u64,
    pub total_blocks: u64,
    pub available_inodes: u64,
    pub total_inodes: u64,
}

pub type HealthReader = Arc<dyn Fn(&Path) -> Result<StorageCapacity> + Send + Sync + 'static>;

pub fn read_storage_capacity(path: &Path) -> Result<StorageCapacity> {
    let path = CString::new(path.as_os_str().as_bytes()).context("data directory contains NUL")?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).context("stat node data directory");
    }
    let stat = unsafe { stat.assume_init() };
    Ok(StorageCapacity {
        available_blocks: stat.f_bavail,
        total_blocks: stat.f_blocks,
        available_inodes: stat.f_favail,
        total_inodes: stat.f_files,
    })
}

pub fn capacity_error(capacity: StorageCapacity) -> Option<String> {
    let disk = ratio(capacity.available_blocks, capacity.total_blocks);
    let inodes = ratio(capacity.available_inodes, capacity.total_inodes);
    match (disk, inodes) {
        (None, _) => Some("node storage health unavailable: zero filesystem blocks".into()),
        (_, None) => Some("node storage health unavailable: zero filesystem inodes".into()),
        (Some(value), _) if value < MIN_AVAILABLE_RATIO => Some(format!(
            "node storage low: {:.1}% blocks available",
            value * 100.0
        )),
        (_, Some(value)) if value < MIN_AVAILABLE_RATIO => Some(format!(
            "node storage low: {:.1}% inodes available",
            value * 100.0
        )),
        _ => None,
    }
}

fn ratio(available: u64, total: u64) -> Option<f64> {
    (total != 0).then_some(available as f64 / total as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_capacity_below_twenty_percent_is_unhealthy() {
        let capacity = |blocks, inodes| StorageCapacity {
            available_blocks: blocks,
            total_blocks: 100,
            available_inodes: inodes,
            total_inodes: 100,
        };
        assert!(capacity_error(capacity(19, 100))
            .unwrap()
            .contains("blocks"));
        assert!(capacity_error(capacity(100, 19))
            .unwrap()
            .contains("inodes"));
        assert_eq!(capacity_error(capacity(20, 20)), None);
    }
}
