//! `@path` file-mention expansion for user prompts.
//!
//! A mention is an `@` at a token boundary (start of text or after
//! whitespace) followed by a relative path under the session's working
//! directory. When the candidate exists on disk the token is rewritten to
//! the absolute path BEFORE the user message is recorded, so the stored
//! transcript and the model request carry full paths while the composer
//! keeps the short `@relative/path` form. Non-path tokens — emails
//! (`a@b.com`), unknown names (`@param`) — pass through verbatim: only
//! existing paths are rewritten, and absolute (`@/etc/passwd`) or
//! escaping (`@../x`) candidates are never treated as mentions.

use std::path::Path;

/// Characters allowed in a mention's path candidate.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

/// Expand every resolvable `@relative/path` mention in `text` against
/// `workdir`. Pure function: no I/O besides existence checks (`try_exists`),
/// never panics on missing paths.
pub fn expand_mentions(text: &str, workdir: &Path) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let at_boundary = i == 0 || chars[i - 1].is_whitespace();
        if c != '@' || !at_boundary {
            out.push(c);
            i += 1;
            continue;
        }
        // Greedy run of path characters after the '@'.
        let mut j = i + 1;
        while j < chars.len() && is_path_char(chars[j]) {
            j += 1;
        }
        let candidate: String = chars[i + 1..j].iter().collect();
        match resolve_candidate(&candidate, workdir) {
            Some((abs, consumed)) => {
                out.push_str(&abs);
                // Re-emit whatever followed the recognized path (e.g.
                // sentence punctuation) verbatim.
                out.extend(&chars[i + 1 + consumed..j]);
                i = j;
            }
            None => {
                // Not a mention: emit the '@' literally and continue; the
                // following characters are copied by later iterations.
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Try `candidate` — and, failing that, `candidate` minus trailing sentence
/// punctuation — against `workdir`. Returns the absolute path plus the
/// number of candidate chars it covers.
fn resolve_candidate(candidate: &str, workdir: &Path) -> Option<(String, usize)> {
    let mut cand = candidate;
    loop {
        if let Some(abs) = existing_abs(workdir, cand) {
            return Some((abs, cand.chars().count()));
        }
        // Trim one trailing punctuation char and retry, so `@file.` at a
        // sentence end still resolves to the file.
        let trimmed = match cand.chars().next_back() {
            Some(t @ ('.' | ',' | ';' | ':' | '!' | '?')) => &cand[..cand.len() - t.len_utf8()],
            _ => return None,
        };
        if trimmed.is_empty() {
            return None;
        }
        cand = trimmed;
    }
}

/// Absolute path for `rel` when it exists under `workdir`. Rejects
/// absolute and `..`-escaping candidates: mentions are strictly relative
/// sub-paths of the workdir (`Path::join` would otherwise REPLACE the
/// workdir for an absolute argument).
fn existing_abs(workdir: &Path, rel: &str) -> Option<String> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel
            .split('/')
            .any(|seg| seg == ".." || seg == "." || seg.is_empty())
    {
        return None;
    }
    let joined = workdir.join(rel);
    // try_exists: a broken symlink reports Err, not false — treat Err as
    // "missing" rather than failing the prompt.
    if !joined.try_exists().unwrap_or(false) {
        return None;
    }
    let abs = std::fs::canonicalize(&joined).unwrap_or(joined);
    Some(abs.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("src/tools")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("src/tools/util.rs"), "x").unwrap();
        let abs = dir.path().canonicalize().unwrap();
        (dir, abs)
    }

    #[test]
    fn expands_existing_file_and_keeps_surrounding_text() {
        let (_d, abs) = setup();
        let out = expand_mentions("read @notes.md please", &abs);
        assert_eq!(out, format!("read {}/notes.md please", abs.display()));
    }

    #[test]
    fn expands_nested_file_and_directory() {
        let (_d, abs) = setup();
        assert_eq!(
            expand_mentions("@src/main.rs", &abs),
            format!("{}/src/main.rs", abs.display())
        );
        assert_eq!(
            expand_mentions("@src", &abs),
            format!("{}/src", abs.display())
        );
    }

    #[test]
    fn expands_multiple_mentions() {
        let (_d, abs) = setup();
        let out = expand_mentions("@notes.md and @src/main.rs", &abs);
        assert_eq!(
            out,
            format!(
                "{}/notes.md and {}/src/main.rs",
                abs.display(),
                abs.display()
            )
        );
    }

    #[test]
    fn leading_mention_at_start_of_text() {
        let (_d, abs) = setup();
        assert_eq!(
            expand_mentions("@notes.md", &abs),
            format!("{}/notes.md", abs.display())
        );
    }

    #[test]
    fn nonexistent_token_stays_verbatim() {
        let (_d, abs) = setup();
        assert_eq!(
            expand_mentions("fix @nope.txt today", &abs),
            "fix @nope.txt today"
        );
    }

    #[test]
    fn email_is_not_a_mention() {
        let (_d, abs) = setup();
        // '@' mid-token (after a non-whitespace char) never triggers.
        assert_eq!(
            expand_mentions("mail a@b.com now", &abs),
            "mail a@b.com now"
        );
        // The domain part alone resolves? b.com must NOT exist here.
        assert_eq!(expand_mentions("mail @b.com now", &abs), "mail @b.com now");
    }

    #[test]
    fn at_param_like_token_stays_verbatim() {
        let (_d, abs) = setup();
        assert_eq!(expand_mentions("see @param docs", &abs), "see @param docs");
    }

    #[test]
    fn sentence_punctuation_after_mention_is_preserved() {
        let (_d, abs) = setup();
        let out = expand_mentions("see @notes.md.", &abs);
        assert_eq!(out, format!("see {}/notes.md.", abs.display()));
        let out = expand_mentions("see @notes.md, ok?", &abs);
        assert_eq!(out, format!("see {}/notes.md, ok?", abs.display()));
    }

    #[test]
    fn absolute_and_escaping_candidates_are_ignored() {
        let (_d, abs) = setup();
        assert_eq!(expand_mentions("@/etc/passwd", &abs), "@/etc/passwd");
        assert_eq!(expand_mentions("@../notes.md", &abs), "@../notes.md");
    }

    #[test]
    fn bare_at_and_plain_text_unchanged() {
        let (_d, abs) = setup();
        assert_eq!(expand_mentions("@", &abs), "@");
        assert_eq!(
            expand_mentions("no mentions here", &abs),
            "no mentions here"
        );
        assert_eq!(expand_mentions("", &abs), "");
    }

    #[test]
    fn punctuation_only_candidate_does_not_loop_or_trim_to_match() {
        let (_d, abs) = setup();
        // Candidate trims down to empty without ever existing.
        assert_eq!(expand_mentions("@...", &abs), "@...");
    }
}
