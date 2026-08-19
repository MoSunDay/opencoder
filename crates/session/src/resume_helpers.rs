//! Pure helpers for session recovery that are small enough not to warrant a
//! home in the larger `resume` module (kept under the 800-line file cap).

/// Infer active skill names from a skill prompt body by matching known
/// skill body prefixes. Used on resume to restore latent tool unlocking.
pub fn infer_skill_names(body: &Option<String>) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut names = HashSet::new();
    if let Some(b) = body {
        let prefix = b.chars().take(200).collect::<String>();
        if prefix.contains("ssh_pty") || prefix.contains("ssh-pty") {
            names.insert("ssh-pty".to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn infer_skill_names_none_body() {
        let names = infer_skill_names(&None);
        assert!(names.is_empty());
    }

    #[test]
    fn infer_skill_names_empty_body() {
        let names = infer_skill_names(&Some(String::new()));
        assert!(names.is_empty());
    }

    #[test]
    fn infer_skill_names_detects_ssh_pty() {
        let body = Some("Use ssh_pty to connect to the server".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(names, HashSet::from(["ssh-pty".to_string()]));
    }

    #[test]
    fn infer_skill_names_detects_ssh_pty_dash() {
        let body = Some("Active skill: ssh-pty".to_string());
        let names = infer_skill_names(&body);
        assert_eq!(names, HashSet::from(["ssh-pty".to_string()]));
    }

    #[test]
    fn infer_skill_names_ignores_after_200_chars() {
        // Content after the first 200 chars should be ignored.
        let padding = "x".repeat(200);
        let body = Some(format!("{padding}ssh_pty"));
        let names = infer_skill_names(&body);
        assert!(
            names.is_empty(),
            "skill names past 200 chars should be ignored"
        );
    }
}
