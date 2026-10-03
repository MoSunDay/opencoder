//! Fail-closed local storage health used for placement and admission.

use anyhow::{Context, Result};
#[cfg(unix)]
use std::{ffi::CString, os::unix::ffi::OsStrExt};
use std::{path::Path, sync::Arc};

pub const MIN_AVAILABLE_BLOCK_RATIO: f64 = 0.10;
pub const MIN_AVAILABLE_INODE_RATIO: f64 = 0.20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StorageCapacity {
    pub available_blocks: u64,
    pub total_blocks: u64,
    pub available_inodes: Option<u64>,
    pub total_inodes: Option<u64>,
}

pub type HealthReader = Arc<dyn Fn(&Path) -> Result<StorageCapacity> + Send + Sync + 'static>;

#[cfg(unix)]
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
        available_inodes: Some(stat.f_favail),
        total_inodes: Some(stat.f_files),
    })
}

pub fn capacity_error(capacity: StorageCapacity) -> Option<String> {
    let Some(disk) = ratio(capacity.available_blocks, capacity.total_blocks) else {
        return Some("node storage health unavailable: zero filesystem blocks".into());
    };
    if disk < MIN_AVAILABLE_BLOCK_RATIO {
        return Some(format!(
            "node storage low: {:.1}% blocks available",
            disk * 100.0
        ));
    }
    match (capacity.available_inodes, capacity.total_inodes) {
        (None, None) => None,
        (Some(available), Some(total)) => match ratio(available, total) {
            None => Some("node storage health unavailable: zero filesystem inodes".into()),
            Some(value) if value < MIN_AVAILABLE_INODE_RATIO => Some(format!(
                "node storage low: {:.1}% inodes available",
                value * 100.0
            )),
            _ => None,
        },
        _ => Some("node storage health unavailable: incomplete inode capacity".into()),
    }
}

fn ratio(available: u64, total: u64) -> Option<f64> {
    (total != 0).then_some(available as f64 / total as f64)
}

#[cfg(windows)]
pub fn read_storage_capacity(path: &Path) -> Result<StorageCapacity> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let path = std::fs::canonicalize(path)?;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let (mut available, mut total, mut free) = (0, 0, 0);
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut available, &mut total, &mut free) } == 0 {
        return Err(std::io::Error::last_os_error()).context("read node disk capacity");
    }
    Ok(StorageCapacity {
        available_blocks: available,
        total_blocks: total,
        available_inodes: None,
        total_inodes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_requires_ten_percent_blocks_and_twenty_percent_inodes() {
        let capacity = |blocks, inodes| StorageCapacity {
            available_blocks: blocks,
            total_blocks: 100,
            available_inodes: Some(inodes),
            total_inodes: Some(100),
        };
        assert!(capacity_error(capacity(9, 100)).unwrap().contains("blocks"));
        assert!(capacity_error(capacity(100, 19))
            .unwrap()
            .contains("inodes"));
        assert_eq!(capacity_error(capacity(10, 20)), None);
        assert_eq!(capacity_error(capacity(19, 100)), None);
    }

    #[test]
    fn filesystems_without_inodes_use_disk_capacity_and_partial_values_fail_closed() {
        let capacity = StorageCapacity {
            available_blocks: 80,
            total_blocks: 100,
            available_inodes: None,
            total_inodes: None,
        };
        assert_eq!(capacity_error(capacity), None);
        assert!(capacity_error(StorageCapacity {
            available_blocks: 9,
            ..capacity
        })
        .unwrap()
        .contains("blocks"));
        assert!(capacity_error(StorageCapacity {
            total_inodes: Some(100),
            ..capacity
        })
        .unwrap()
        .contains("incomplete inode"));
    }

    #[test]
    fn unknown_storage_capacity_remains_unhealthy() {
        let capacity = StorageCapacity {
            available_blocks: 0,
            total_blocks: 0,
            available_inodes: Some(0),
            total_inodes: Some(0),
        };
        assert!(capacity_error(capacity)
            .unwrap()
            .contains("zero filesystem blocks"));
        assert!(capacity_error(StorageCapacity {
            available_blocks: 100,
            total_blocks: 100,
            ..capacity
        })
        .unwrap()
        .contains("zero filesystem inodes"));
    }
}
