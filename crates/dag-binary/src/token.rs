pub fn parse_resource_token(token: &str) -> Option<(String, Option<u32>)> {
    let (name, version) = match token.split_once('@') {
        None => (token, None),
        Some((name, pin)) => {
            let digits = pin.strip_prefix('v')?;
            if digits.is_empty()
                || digits.starts_with('0')
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            (name, Some(digits.parse().ok()?))
        }
    };
    crate::validate_name(name).ok()?;
    Some((name.into(), version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_and_immutable_versions_parse() {
        assert_eq!(parse_resource_token("tool"), Some(("tool".into(), None)));
        assert_eq!(
            parse_resource_token("tool@v3"),
            Some(("tool".into(), Some(3)))
        );
        assert_eq!(
            parse_resource_token("a.b@v4294967295"),
            Some(("a.b".into(), Some(u32::MAX)))
        );
    }

    #[test]
    fn unsafe_names_and_noncanonical_versions_fail() {
        for token in [
            "",
            "..",
            "../tool",
            "a/b",
            "a b",
            "tool@v0",
            "tool@v01",
            "tool@v4294967296",
            "tool@v3@v4",
            "tool@3",
        ] {
            assert_eq!(parse_resource_token(token), None, "{token}");
        }
    }
}
