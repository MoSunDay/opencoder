//! Spec-side module-token grammar: how a wasm step's command names a
//! pool module.
//!
//! A command's first whitespace token is the module reference. Two
//! pool-shaped forms exist — everything else is out-of-band:
//!
//! - `tool.wasm` — pin the pool's `current` version at accept time.
//! - `tool@v3.wasm` — pin an EXPLICIT version. Version directories are
//!   immutable (never deleted, never renumbered, even by rollback), so
//!   the pin freezes exact bytes regardless of later `current` flips.
//!
//! Pure grammar check only: no filesystem access, no pool knowledge.

/// Parse a module token into pool coordinates `(name, version)`:
/// `tool.wasm` → `("tool", None)` (current), `tool@v3.wasm` →
/// `("tool", Some(3))`. `None` when the token is not pool-shaped: a
/// missing (or whole-token) `.wasm` suffix, a stem failing
/// [`crate::validate_name`] (nested or traversing paths can never come
/// from the flat pool), or a malformed `@v<n>` pin — `v0`, leading
/// zeros, non-digits, stray `@`, u32 overflow.
pub fn parse_module_token(token: &str) -> Option<(String, Option<u32>)> {
    let stem = token.strip_suffix(".wasm")?;
    if stem.is_empty() {
        return None;
    }
    let (name, version) = match stem.split_once('@') {
        None => (stem, None),
        Some((name, pin)) => (name, Some(parse_version_suffix(pin)?)),
    };
    crate::validate_name(name).ok()?;
    Some((name.to_string(), version))
}

/// `v<n>` with `n ≥ 1`, no leading zeros, within u32 (version numbers
/// are `u32` in pool metas; a longer digit run can never match one).
fn parse_version_suffix(pin: &str) -> Option<u32> {
    let digits = pin.strip_prefix('v')?;
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    if canonical {
        digits.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpinned_tokens_map_to_a_pool_name() {
        assert_eq!(
            parse_module_token("tool.wasm"),
            Some(("tool".to_string(), None))
        );
        assert_eq!(
            parse_module_token("my_tool-2.wasm"),
            Some(("my_tool-2".to_string(), None))
        );
        assert_eq!(
            parse_module_token("a.b.wasm"),
            Some(("a.b".to_string(), None))
        );
    }

    #[test]
    fn pinned_tokens_map_to_name_and_version() {
        assert_eq!(
            parse_module_token("tool@v3.wasm"),
            Some(("tool".to_string(), Some(3)))
        );
        assert_eq!(
            parse_module_token("tool@v1.wasm"),
            Some(("tool".to_string(), Some(1)))
        );
        assert_eq!(
            parse_module_token("a.b@v4294967295.wasm"),
            Some(("a.b".to_string(), Some(u32::MAX)))
        );
    }

    #[test]
    fn non_pool_tokens_are_rejected() {
        for token in [
            "tool", "tool.bin", ".wasm", "tool@v3", "../x.wasm", "build/out.wasm",
        ] {
            assert_eq!(parse_module_token(token), None, "{token}");
        }
    }

    #[test]
    fn malformed_version_pins_are_rejected() {
        for token in [
            "tool@.wasm", "@v3.wasm", "tool@v.wasm", "tool@vx.wasm", "tool@3.wasm",
            "tool@v0.wasm", "tool@v01.wasm", "tool@v4294967296.wasm", "tool@v3@v4.wasm",
            "tool@@v3.wasm",
        ] {
            assert_eq!(parse_module_token(token), None, "{token}");
        }
    }
}
