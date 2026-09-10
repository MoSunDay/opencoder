//! Validation for pool names and raw wasm module bytes — the gate every
//! write passes through before touching the filesystem.

use crate::MAX_WASM_BYTES;

/// Max pool name length — same rule as agent/resource names.
const MAX_NAME_LEN: usize = 48;

/// Validate a wasm pool name: same charset/length rules as agent and
/// resource names (`opencode_core::agent`), with no reserved set — pools
/// live directly under their own root, so nothing can collide. Error
/// style mirrors `envs.rs` (Chinese, same wording as resource names).
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("名称过长（>{MAX_NAME_LEN} 字符）"));
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".to_string());
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.');
    if !ok {
        return Err("只能包含字母、数字、_、-、.".to_string());
    }
    Ok(())
}

/// Validate raw wasm module bytes against an explicit size cap: the
/// module must be non-empty, carry at least the 8-byte header
/// (`\0asm` magic + little-endian u32 version), declare version
/// exactly 1, and stay within `cap` bytes. English error strings — this
/// is a byte-level wire contract, unlike the human-facing name errors.
pub fn validate_wasm_bytes_with_cap(bytes: &[u8], cap: usize) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("wasm module is empty".to_string());
    }
    if bytes.len() < 8 {
        return Err(format!(
            "wasm module too short: {} bytes, need at least 8 (magic + version)",
            bytes.len()
        ));
    }
    if bytes[0..4] != *b"\0asm" {
        let got: String = bytes[0..4].iter().map(|b| format!("{b:02x}")).collect();
        return Err(format!("bad wasm magic: expected \\0asm, got 0x{got}"));
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 1 {
        return Err(format!(
            "unsupported wasm version: expected 1, got {version}"
        ));
    }
    if bytes.len() > cap {
        return Err(format!(
            "wasm module too large: {} bytes exceeds the {cap} byte cap",
            bytes.len()
        ));
    }
    Ok(())
}

/// [`validate_wasm_bytes_with_cap`] under the pool's 32 MiB cap
/// ([`crate::MAX_WASM_BYTES`]) — the default gate for every save.
pub fn validate_wasm_bytes(bytes: &[u8]) -> Result<(), String> {
    validate_wasm_bytes_with_cap(bytes, MAX_WASM_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimal valid module: header only.
    const HEADER: &[u8] = b"\0asm\x01\0\0\0";

    #[test]
    fn name_matrix() {
        for name in [
            "a",
            "A1",
            "wasm-sort",
            "mod.v2",
            "under_score",
            &"x".repeat(48),
        ] {
            assert_eq!(validate_name(name), Ok(()), "expected ok: {name}");
        }
        for name in [
            "",
            ".",
            "..",
            &"x".repeat(49),
            "sp ace",
            "中文",
            "a/b",
            "a\\b",
        ] {
            assert!(validate_name(name).is_err(), "expected err: {name}");
        }
        // Exact wording spot-checks (mirrors the resource-name errors).
        assert_eq!(validate_name(""), Err("名称不能为空".to_string()));
        assert_eq!(validate_name(".."), Err("名称不能是 . 或 ..".to_string()));
        assert_eq!(
            validate_name("sp ace"),
            Err("只能包含字母、数字、_、-、.".to_string())
        );
        assert_eq!(
            validate_name(&"x".repeat(49)),
            Err("名称过长（>48 字符）".to_string())
        );
    }

    #[test]
    fn valid_header_and_bad_magic() {
        assert_eq!(validate_wasm_bytes_with_cap(HEADER, 16), Ok(()));
        // Header + payload still fine under the cap.
        let with_payload = [HEADER, b"(module)"].concat();
        assert_eq!(validate_wasm_bytes_with_cap(&with_payload, 16), Ok(()));
        // Wrong magic (leading NUL dropped → shifted bytes).
        let bad_magic = b"asm\x00\x01\0\0\0";
        assert!(validate_wasm_bytes_with_cap(bad_magic, 16)
            .unwrap_err()
            .contains("magic"));
    }

    #[test]
    fn short_empty_and_bad_version_rejected() {
        assert!(validate_wasm_bytes_with_cap(&[], 16)
            .unwrap_err()
            .contains("empty"));
        assert!(validate_wasm_bytes_with_cap(b"\0as", 16)
            .unwrap_err()
            .contains("too short"));
        assert!(validate_wasm_bytes_with_cap(b"\0asm", 16)
            .unwrap_err()
            .contains("too short"));
        // Version 2 in the LE version field → rejected.
        let bad_version = b"\0asm\x02\0\0\0";
        assert!(validate_wasm_bytes_with_cap(bad_version, 16)
            .unwrap_err()
            .contains("version"));
    }

    #[test]
    fn cap_enforced_and_default_cap_is_32mib() {
        // 9 bytes against an 8-byte cap → too large.
        let too_big = [HEADER, b"x"].concat();
        assert!(validate_wasm_bytes_with_cap(&too_big, 8)
            .unwrap_err()
            .contains("too large"));
        assert_eq!(validate_wasm_bytes_with_cap(&too_big, 9), Ok(()));
        // The default gate is the 32 MiB pool cap.
        assert_eq!(MAX_WASM_BYTES, 32 * 1024 * 1024);
        assert_eq!(validate_wasm_bytes(HEADER), Ok(()));
    }
}
