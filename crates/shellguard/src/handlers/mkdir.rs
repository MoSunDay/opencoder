//! `mkdir` operand scoping: every operand must resolve into a declared safe
//! scope or a built-in release directory (the cwd is NOT a writable scope).
//!
//! Ported from rippy (MIT) https://github.com/mpecan/rippy

use std::path::Path;

use super::{
    canonicalize_existing_ancestor, is_within_safe_dir, normalize_path, Classification, Handler,
    HandlerContext,
};
use crate::verdict::AllowReason;

pub(crate) static MKDIR_HANDLER: MkdirHandler = MkdirHandler;

pub(crate) struct MkdirHandler;

/// Flags that take a value argument (skip both flag and value).
const VALUE_FLAGS: &[&str] = &["-m", "--mode"];

impl Handler for MkdirHandler {
    fn commands(&self) -> &[&str] {
        &["mkdir"]
    }

    fn classify(&self, ctx: &HandlerContext) -> Classification {
        if ctx.remote {
            return Classification::Ask("mkdir in remote context".into());
        }

        let mut i = 0;
        let mut has_targets = false;

        while i < ctx.args.len() {
            let arg = &ctx.args[i];

            // Skip flags
            if arg.starts_with('-') {
                if VALUE_FLAGS.contains(&arg.as_str()) {
                    i += 1; // skip the value too
                }
                i += 1;
                continue;
            }

            has_targets = true;

            // Can't statically resolve
            if arg.contains('$') || arg.contains('`') {
                return Classification::Ask("mkdir with variable expansion".into());
            }

            if arg.starts_with('~') {
                return Classification::Ask(format!("mkdir in home directory ({arg})"));
            }

            // Scope check with symlink hardening (#F10): the logical path
            // must sit in the release set (plus declared scopes), and so must
            // its symlink-resolved real path — a link planted under /tmp
            // pointing elsewhere must not launder the write.
            let resolved = if Path::new(arg.as_str()).is_absolute() {
                normalize_path(Path::new(arg.as_str()))
            } else {
                normalize_path(&ctx.working_directory.join(arg.as_str()))
            };
            let canonical = canonicalize_existing_ancestor(&resolved);
            if !is_within_safe_dir(&resolved, ctx.safe_scopes)
                || !is_within_safe_dir(&canonical, ctx.safe_scopes)
            {
                return Classification::Ask(format!("mkdir outside allowed scope ({arg})"));
            }

            i += 1;
        }

        if has_targets {
            Classification::Allow(AllowReason::ReleasedWrite(
                "mkdir within allowed scope".into(),
            ))
        } else {
            Classification::Ask("mkdir (no directory specified)".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn is_allow(c: &Classification) -> bool {
        matches!(c, Classification::Allow(_))
    }

    fn is_ask(c: &Classification) -> bool {
        matches!(c, Classification::Ask(_))
    }

    // sandbox: cwd is not a release set
    #[test]
    fn mkdir_relative_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string(), "src/new_dir".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    // sandbox: cwd is not a release set
    #[test]
    fn mkdir_absolute_in_project_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["/project/build".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_tmp_allows() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string(), "/tmp/build-output".to_string()];
        assert!(is_allow(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_outside_project_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["/etc/new_dir".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_config_allowed_dir() {
        let cwd = PathBuf::from("/project");
        let allowed = vec![PathBuf::from("/opt/repos")];
        let args = ["-p".to_string(), "/opt/repos/new-project".to_string()];
        assert!(is_allow(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            safe_scopes: &allowed,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_variable_expansion_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["$HOME/new_dir".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_tilde_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["~/new_dir".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_no_args_asks() {
        let cwd = PathBuf::from("/project");
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &[])
        })));
    }

    #[test]
    fn mkdir_flags_only_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    // sandbox: cwd is not a release set
    #[test]
    fn mkdir_mode_flag_target_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["-m".to_string(), "755".to_string(), "src/build".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    // sandbox: cwd is not a release set
    #[test]
    fn mkdir_multiple_dirs_in_cwd_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string(), "src/a".to_string(), "src/b".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_multiple_dirs_one_unsafe() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string(), "src/a".to_string(), "/etc/b".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    #[test]
    fn mkdir_remote_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["src/dir".to_string()];
        let ctx = HandlerContext {
            working_directory: &cwd,
            remote: true,
            ..HandlerContext::test("mkdir", &args)
        };
        assert!(is_ask(&MKDIR_HANDLER.classify(&ctx)));
    }

    #[test]
    fn mkdir_dotdot_escape_asks() {
        let cwd = PathBuf::from("/project");
        let args = ["-p".to_string(), "../../etc/evil".to_string()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: &cwd,
            ..HandlerContext::test("mkdir", &args)
        })));
    }

    /// #F10: a symlink planted inside the release set must not launder a
    /// mkdir outside it — the operand's real (canonicalized) path decides.
    #[test]
    fn mkdir_through_a_planted_symlink_asks() {
        // NOT std::env::temp_dir(): that honors TMPDIR and may live outside
        // the release set this policy hardcodes.
        let seq = || {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            std::path::PathBuf::from(format!("/tmp/shellguard-mkdir-symlink-{}-{n}", std::process::id()))
        };
        let dir = seq();
        assert!(std::fs::create_dir_all(&dir).is_ok(), "fixture: {dir:?}");
        // A real subdir under the release dir still allows.
        assert!(std::fs::create_dir(dir.join("real")).is_ok());
        let real_args = ["-p".to_string(), dir.join("real/sub").to_string_lossy().into_owned()];
        assert!(is_allow(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: Path::new("/project"),
            ..HandlerContext::test("mkdir", &real_args)
        })));

        // A link under the release dir pointing OUTSIDE it: the logical path
        // looks released, the real path does not -> Ask. (`/etc` exists, so
        // the planted link resolves.)
        assert!(
            std::os::unix::fs::symlink("/etc", dir.join("evil")).is_ok(),
            "fixture: failed to plant symlink"
        );
        let evil_args = ["-p".to_string(), dir.join("evil/sub").to_string_lossy().into_owned()];
        assert!(is_ask(&MKDIR_HANDLER.classify(&HandlerContext {
            working_directory: Path::new("/project"),
            ..HandlerContext::test("mkdir", &evil_args)
        })));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
