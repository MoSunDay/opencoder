use crate::MAX_BINARY_BYTES;

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    if name.len() > 48 {
        return Err("名称过长（>48 字符）".into());
    }
    if name.starts_with('.') {
        return Err("名称不能以 . 开头".into());
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err("只能包含字母、数字、_、-、.".into());
    }
    Ok(())
}

pub fn validate_binary_bytes_with_cap(bytes: &[u8], cap: usize) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("binary is empty".into());
    }
    if bytes.len() > cap {
        return Err(format!(
            "binary too large: {} bytes exceeds the {cap} byte cap",
            bytes.len()
        ));
    }
    if bytes.len() < 64 {
        return Err("ELF header too short".into());
    }
    if bytes[..4] != *b"\x7fELF" {
        return Err("bad ELF magic".into());
    }
    if bytes[4..7] != [2, 1, 1] {
        return Err("binary must be little-endian ELF64 version 1".into());
    }
    let kind = u16::from_le_bytes(bytes[16..18].try_into().unwrap());
    let machine = u16::from_le_bytes(bytes[18..20].try_into().unwrap());
    if !matches!(kind, 2 | 3) || !matches!(machine, 62 | 183) {
        return Err("binary must be an x86_64 or aarch64 executable".into());
    }
    let offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let entry_size = u16::from_le_bytes(bytes[54..56].try_into().unwrap()) as u64;
    let count = u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as u64;
    let end = count
        .checked_mul(entry_size)
        .and_then(|size| offset.checked_add(size));
    if count == 0
        || entry_size != 56
        || offset < 64
        || end.is_none_or(|end| end > bytes.len() as u64)
    {
        return Err("invalid ELF program headers".into());
    }
    let mut has_load = false;
    for index in 0..count {
        let start = (offset + index * entry_size) as usize;
        let kind = u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
        let file_offset = u64::from_le_bytes(bytes[start + 8..start + 16].try_into().unwrap());
        let file_size = u64::from_le_bytes(bytes[start + 32..start + 40].try_into().unwrap());
        if file_offset
            .checked_add(file_size)
            .is_none_or(|end| end > bytes.len() as u64)
        {
            return Err("ELF segment exceeds binary size".into());
        }
        has_load |= kind == 1;
    }
    if !has_load {
        return Err("ELF executable has no load segment".into());
    }
    Ok(())
}

pub fn validate_binary_bytes(bytes: &[u8]) -> Result<(), String> {
    validate_binary_bytes_with_cap(bytes, MAX_BINARY_BYTES)
}

pub fn validate_host_architecture(bytes: &[u8]) -> Result<(), String> {
    validate_binary_bytes(bytes)?;
    let expected = match std::env::consts::ARCH {
        "x86_64" => 62,
        "aarch64" => 183,
        architecture => return Err(format!("unsupported node architecture {architecture}")),
    };
    let actual = u16::from_le_bytes(bytes[18..20].try_into().unwrap());
    if actual != expected {
        return Err("binary architecture does not match the executing node".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable() -> Vec<u8> {
        let mut bytes = vec![0; 120];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16] = 2;
        bytes[18] = 62;
        bytes[32] = 64;
        bytes[54] = 56;
        bytes[56] = 1;
        bytes[64] = 1;
        bytes
    }

    #[test]
    fn executable_header_and_size_are_checked() {
        let bytes = executable();
        assert!(validate_binary_bytes_with_cap(&bytes, 120).is_ok());
        assert!(validate_binary_bytes_with_cap(&bytes, 119)
            .unwrap_err()
            .contains("too large"));
        assert!(validate_binary_bytes(b"text")
            .unwrap_err()
            .contains("short"));
        let mut invalid = bytes.clone();
        invalid[4] = 1;
        assert!(validate_binary_bytes(&invalid).is_err());
        invalid = bytes;
        invalid[32..40].fill(255);
        assert!(validate_binary_bytes(&invalid)
            .unwrap_err()
            .contains("program headers"));
    }

    #[test]
    fn pool_names_are_path_safe() {
        for name in ["a", "A1", "under_score", "tool.v2"] {
            assert!(validate_name(name).is_ok());
        }
        for name in ["", ".", "..", ".locks", "a/b", "a b", &"x".repeat(49)] {
            assert!(validate_name(name).is_err());
        }
    }
}
