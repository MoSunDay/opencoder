//! Validate complete committed SQLite WAL frames before querying their copy.
//! The WAL index (-shm) contains only a derived index, not database records.
use anyhow::{ensure, Result};
use std::{fs, path::Path};

pub(super) fn companion(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

pub(super) fn sidecar(path: &Path) -> Result<bool> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(false);
    };
    let Some(base) = name
        .strip_suffix("-shm")
        .or_else(|| name.strip_suffix("-wal"))
    else {
        return Ok(false);
    };
    let base = path.with_file_name(base);
    let Ok(meta) = fs::symlink_metadata(&base) else {
        return Ok(false);
    };
    if !meta.file_type().is_file() || meta.len() > 4 * 1024 * 1024 {
        return Ok(false);
    }
    use std::io::Read;
    let mut header = [0; 16];
    if fs::File::open(base)?.read_exact(&mut header).is_err() {
        return Ok(false);
    }
    if header != *b"SQLite format 3\0" {
        return Ok(false);
    }
    if name.ends_with("-shm") {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.len() < 32768 || metadata.len() > 4 * 1024 * 1024 || metadata.len() % 32768 != 0
        {
            return Ok(false);
        }
        let mut index = [0; 96];
        fs::File::open(path)?.read_exact(&mut index)?;
        if index[..48] != index[48..]
            || u32::from_ne_bytes(index[..4].try_into()?) != 3_007_000
            || index[12] != 1
        {
            return Ok(false);
        }
        let sum = checksum(&index[..40], cfg!(target_endian = "little"), [0, 0]);
        if sum
            != [
                u32::from_ne_bytes(index[40..44].try_into()?),
                u32::from_ne_bytes(index[44..48].try_into()?),
            ]
        {
            return Ok(false);
        }
    }
    // The full directory walk must also validate/query this base and its WAL.
    Ok(true)
}

fn integer(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("four bytes"))
}
fn checksum(bytes: &[u8], little: bool, mut sum: [u32; 2]) -> [u32; 2] {
    for pair in bytes.as_chunks::<8>().0 {
        let word = |bytes: &[u8]| {
            if little {
                u32::from_le_bytes(bytes.try_into().unwrap())
            } else {
                integer(bytes)
            }
        };
        sum[0] = sum[0].wrapping_add(word(&pair[..4])).wrapping_add(sum[1]);
        sum[1] = sum[1].wrapping_add(word(&pair[4..])).wrapping_add(sum[0]);
    }
    sum
}

pub(super) fn validate(bytes: &[u8], database: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    ensure!(
        bytes.len() >= 32 && database.len() >= 100,
        "truncated resource WAL"
    );
    let magic = integer(&bytes[..4]);
    ensure!(
        matches!(magic, 0x377f0682 | 0x377f0683),
        "unknown resource WAL format"
    );
    ensure!(
        integer(&bytes[4..8]) == 3_007_000,
        "unknown resource WAL version"
    );
    let page = integer(&bytes[8..12]) as usize;
    let encoded = u16::from_be_bytes(database[16..18].try_into()?);
    let expected = if encoded == 1 {
        65536
    } else {
        encoded as usize
    };
    ensure!(
        page == expected && page.is_power_of_two() && (512..=65536).contains(&page),
        "invalid resource WAL page size"
    );
    ensure!(
        (bytes.len() - 32).is_multiple_of(24 + page),
        "incomplete resource WAL frame"
    );
    let little = magic == 0x377f0682;
    let mut sum = checksum(&bytes[..24], little, [0, 0]);
    ensure!(
        sum == [integer(&bytes[24..28]), integer(&bytes[28..32])],
        "invalid resource WAL header checksum"
    );
    let mut committed = true;
    for frame in bytes[32..].chunks_exact(24 + page) {
        ensure!(
            integer(&frame[..4]) > 0 && frame[8..16] == bytes[16..24],
            "invalid resource WAL frame identity"
        );
        sum = checksum(&frame[..8], little, sum);
        sum = checksum(&frame[24..], little, sum);
        ensure!(
            sum == [integer(&frame[16..20]), integer(&frame[20..24])],
            "invalid resource WAL frame checksum"
        );
        committed = integer(&frame[4..8]) != 0;
    }
    ensure!(committed, "resource WAL has an unfinished transaction");
    Ok(())
}
