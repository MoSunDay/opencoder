//! User-authored "skill" instruction packs.
//!
//! A skill is a markdown file whose body is injected into the agent's system
//! prompt when the user activates it from the TUI (`$` menu). This lets users
//! drop reusable operating procedures (a SKILL.md per topic) into
//! `~/.opencoder/skills/` and load them on demand without touching the agent
//! registry or config.
//!
//! Two on-disk layouts are accepted, mirroring the opencoder skill convention:
//!
//! ```text
//! ~/.opencoder/skills/<name>.md
//! ~/.opencoder/skills/<name>/SKILL.md
//! ```
//!
//! Both may carry an optional YAML-ish frontmatter block delimited by `---`:
//!
//! ```text
//! ---
//! name: pretty-name
//! description: one line shown in the picker
//! ---
//! <body instructions>
//! ```
//!
//! When frontmatter is absent the name falls back to the file/dir stem and the
//! description to the first non-empty, non-heading body line.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Cache entry: the (path, mtime) fingerprint captured at discovery time plus
/// the discovered skills behind an Arc so hits clone cheaply.
struct DiscoverCacheEntry {
    files: Vec<(PathBuf, SystemTime)>,
    skills: Arc<Vec<Skill>>,
}

/// Process-level discovery cache, keyed on the single most recently scanned
/// root.
///
/// Purpose: the UI submit path (`skill_persist::resolve_persist`) calls
/// [`discover`] on every Enter/Tab submit, which without this cache means a
/// full `read_to_string` sweep of the skills directory per keypress. With the
/// cache, a hit costs one `read_dir` plus N `stat` calls and never reads a
/// skill file. Invalidation is exact: any (path, mtime) change in the watched
/// file set (see [`fingerprint`]) makes the next call a miss and forces a
/// rescan. [`discover_in`] stays uncached so tests pointing at tempdirs
/// always observe the real directory.
static DISCOVER_CACHE: Mutex<Option<(PathBuf, DiscoverCacheEntry)>> = Mutex::new(None);

mod seed;

pub use seed::{
    seed_builtin_skills, seed_builtin_skills_in, seed_dep_gated_skills,
    seed_dep_gated_skills_in, write_install_script, write_install_script_in,
    DEPS_SENTINEL,
};

/// A loadable skill instruction pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: PathBuf,
}

/// Default discovery root: the binary's own global config home
/// (`~/.opencoder/skills`). Returns `~/.opencoder/skills` only as an absolute
/// fallback when no home directory can be resolved, so discovery never panics.
pub fn skills_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".opencoder").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".opencoder").join("skills"))
}

/// Scan `~/.opencoder/skills` and return every skill found, sorted by name.
///
/// A missing or unreadable directory is not an error — it yields an empty
/// `Vec`, so the TUI picker simply reports "no skills" instead of crashing.
pub fn discover() -> Vec<Skill> {
    discover_cached(&skills_dir())
}

/// Cached variant of [`discover_in`] for hot paths: returns the previously
/// discovered skills when `root` matches the cached root and its
/// [`fingerprint`] is unchanged, otherwise rescans via [`discover_in`] and
/// refreshes the cache. The lock is never held across the rescan itself, so
/// concurrent callers never serialize on file I/O.
pub fn discover_cached(root: &Path) -> Vec<Skill> {
    let files = fingerprint(root);
    {
        let cache = DISCOVER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_root, entry)) = cache.as_ref() {
            if cached_root == root && entry.files == files {
                return entry.skills.as_ref().clone();
            }
        }
    }
    let skills = discover_in(root);
    let mut cache = DISCOVER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *cache = Some((
        root.to_path_buf(),
        DiscoverCacheEntry {
            files,
            skills: Arc::new(skills.clone()),
        },
    ));
    skills
}

/// Directory-scanning core, factored out so tests can point at a tempdir.
pub fn discover_in(root: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let stem = file_stem(&path).to_string();
                if let Some(sk) = parse_skill(&path, &stem) {
                    out.push(sk);
                }
            }
        } else if ft.is_dir() {
            let inner = path.join("SKILL.md");
            if inner.is_file() {
                let stem = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Some(sk) = parse_skill(&inner, &stem) {
                    out.push(sk);
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Build the (path, mtime) fingerprint of exactly the file set
/// [`discover_in`] reads: top-level `*.md` files plus `<dir>/SKILL.md` for
/// each subdirectory that has one. Sorted for stable comparison. An empty
/// result (unreadable root or any stat failure) always mismatches a cached
/// non-empty fingerprint, forcing a rescan.
fn fingerprint(root: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let target = if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                path
            } else {
                continue;
            }
        } else if ft.is_dir() {
            let inner = path.join("SKILL.md");
            if inner.is_file() {
                inner
            } else {
                continue;
            }
        } else {
            continue;
        };
        match std::fs::metadata(&target).and_then(|m| m.modified()) {
            Ok(mtime) => out.push((target, mtime)),
            Err(_) => return Vec::new(),
        }
    }
    out.sort();
    out
}

/// Parse one markdown file into a [`Skill`]. Returns `None` on read error.
pub fn parse_skill(path: &Path, fallback_name: &str) -> Option<Skill> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (front, body) = split_frontmatter(&raw);
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    for (k, v) in front {
        match k.as_str() {
            "name" => {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    name = trimmed.to_string();
                }
            }
            "description" => {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    description = trimmed.to_string();
                }
            }
            _ => {}
        }
    }
    let body_trim = body.trim();
    if description.is_empty() {
        description = first_body_line(body_trim);
    }
    let body_owned = if body_trim.is_empty() {
        raw.trim().to_string()
    } else {
        body_trim.to_string()
    };
    Some(Skill {
        name,
        description,
        body: body_owned,
        source: path.to_path_buf(),
    })
}

/// Split off a leading `---\n...\n---` block. Returns `(pairs, body)` where
/// `pairs` is the frontmatter key/value lines and `body` is everything after.
/// Tolerant: only treats a block as frontmatter when the very first line is
/// exactly `---`.
fn split_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    let mut lines = raw.lines();
    let first = match lines.next() {
        Some(l) => l,
        None => return (Vec::new(), String::new()),
    };
    if first.trim() != "---" {
        return (Vec::new(), raw.to_string());
    }
    let mut pairs = Vec::new();
    for line in lines.by_ref() {
        if line.trim() == "---" {
            // closing fence; remaining lines form the body
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            pairs.push((k.trim().to_string(), v.to_string()));
        }
    }
    // `lines.by_ref()` consumed up to (and including) the closing fence;
    // collect the remainder as the body.
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    (pairs, body)
}

/// First non-empty body line that isn't a markdown heading; used as a
/// description fallback when no frontmatter was supplied.
fn first_body_line(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') {
            continue;
        }
        return t.to_string();
    }
    String::new()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Strip every `$name` token from `text`, returning the cleaned text and the
/// list of skill names in the order they appeared (duplicates are preserved
/// here and deduped by the caller).
///
/// A token is `$` immediately followed by an ASCII lowercase letter, with the
/// name extending over `[a-z0-9-]`. A `$` not followed by a lowercase letter
/// (`$5`, `$HOME`, `$$`, trailing `$`) is literal text. The scan is UTF-8
/// safe: `$` and all name bytes are ASCII, so byte-level detection never splits
/// a multi-byte char.
pub fn extract_skill_tokens(text: &str) -> (String, Vec<String>) {
    let mut clean = String::with_capacity(text.len());
    let mut names = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'$' && i + 1 < text.len() && bytes[i + 1].is_ascii_lowercase() {
            let start = i + 1;
            let mut end = start;
            while end < text.len() {
                let b = bytes[end];
                if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' {
                    end += 1;
                } else {
                    break;
                }
            }
            names.push(text[start..end].to_string());
            i = end;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        clean.push(ch);
        i += ch.len_utf8();
    }
    (clean, names)
}


/// Strip only the `$name` tokens whose name is in `resolved`, leaving every
/// **unresolved** `$name` sequence intact as literal text.
///
/// `extract_skill_tokens` strips *all* tokens (it is used purely to discover
/// and activate skills) — but the resolvers that own the actual `clean` text
/// must not discard bytes for names that matched no skill. Otherwise a token
/// like `$review1) task` (where `review1` is not a real skill, but the greedy
/// `[a-z0-9-]` charset consumed the `1`) permanently deletes the user's
/// content. This function rebuilds the text keeping unresolved `$name` bytes
/// verbatim.
///
/// The scan mirrors [`extract_skill_tokens`]: a token is `$` followed by an
/// ASCII lowercase letter, extending over `[a-z0-9-]`. UTF-8 safe via
/// byte-level detection — `$` and all name bytes are ASCII, so the scan never
/// splits a multi-byte char.
pub fn strip_resolved_skill_tokens(text: &str, resolved: &HashSet<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'$' && i + 1 < text.len() && bytes[i + 1].is_ascii_lowercase() {
            let start = i + 1;
            let mut end = start;
            while end < text.len() {
                let b = bytes[end];
                if b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' {
                    end += 1;
                } else {
                    break;
                }
            }
            if resolved.contains(&text[start..end]) {
                // Resolved token: drop it entirely.
                i = end;
                continue;
            }
            // Unresolved: emit the `$` and let the main loop copy the name
            // bytes verbatim (they are all ASCII lowercase/digit/dash, never
            // `$`, so no nested token can form).
            out.push('$');
            i += 1;
            continue;
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Prefix a skill's body with its source-file path as a blockquote annotation
/// so the injected prompt carries enough context for the agent to locate
/// skill-relative assets (e.g. `EXAMPLES.md`) referenced inside the body.
pub fn body_with_source(skill: &Skill) -> String {
    format!("> Source: {}\n\n{}", skill.source.display(), skill.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn body_with_source_prefixes_path_before_body() {
        let skill = Skill {
            name: "demo".into(),
            description: "d".into(),
            body: "Do the thing.".into(),
            source: PathBuf::from("/skills/demo/SKILL.md"),
        };
        let out = body_with_source(&skill);
        assert!(
            out.starts_with("> Source: /skills/demo/SKILL.md"),
            "must start with source path annotation: {out}"
        );
        assert!(out.contains("Do the thing."), "body must follow the annotation");
    }

    fn write(path: impl AsRef<Path>, contents: &str) {
        let p = path.as_ref();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, contents).unwrap();
    }

    #[test]
    fn parses_frontmatter_name_and_description() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("foo.md");
        write(
            &p,
            "---\nname: Pretty Foo\ndescription: does the foo thing\n---\nbody line one\nbody line two\n",
        );
        let sk = parse_skill(&p, "foo").unwrap();
        assert_eq!(sk.name, "Pretty Foo");
        assert_eq!(sk.description, "does the foo thing");
        assert!(sk.body.contains("body line one"));
        assert!(sk.body.contains("body line two"));
    }

    #[test]
    fn falls_back_to_stem_and_first_line_without_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bar.md");
        write(&p, "# Heading\nfirst real line\nmore\n");
        let sk = parse_skill(&p, "bar").unwrap();
        assert_eq!(sk.name, "bar");
        assert_eq!(sk.description, "first real line");
    }

    #[test]
    fn frontmatter_with_blank_name_keeps_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("baz.md");
        write(&p, "---\nname:   \ndescription: hi\n---\nbody\n");
        let sk = parse_skill(&p, "baz").unwrap();
        assert_eq!(sk.name, "baz");
        assert_eq!(sk.description, "hi");
    }

    #[test]
    fn discover_picks_flat_md_and_nested_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path().join("alpha.md"),
            "---\nname: Alpha\n---\na body\n",
        );
        write(
            dir.path().join("nested").join("SKILL.md"),
            "nested body line\n",
        );
        let found = discover_in(dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Alpha");
        assert_eq!(found[1].name, "nested");
        assert_eq!(found[1].description, "nested body line");
    }

    #[test]
    fn discover_ignores_non_markdown_and_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("notmd.txt"), "nope\n");
        assert!(discover_in(dir.path()).is_empty());
        assert!(discover_in(Path::new("/no/such/dir/here")).is_empty());
    }

    #[test]
    fn discover_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("zeta.md"), "z\n");
        write(dir.path().join("alpha.md"), "a\n");
        write(dir.path().join("mid.md"), "m\n");
        let names: Vec<_> = discover_in(dir.path())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }

    // ----- extract_skill_tokens tests (migrated from tui/skill_token.rs) -----

    #[test]
    fn extract_tokens_empty_input() {
        let (clean, names) = extract_skill_tokens("");
        assert!(clean.is_empty());
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_lone_dollar_is_literal() {
        let (clean, names) = extract_skill_tokens("price is $5");
        assert_eq!(clean, "price is $5");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_basic_stripped() {
        let (clean, names) = extract_skill_tokens("$code");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn extract_tokens_mid_text_preserves_surrounding_text() {
        let (clean, names) = extract_skill_tokens("hello $code world");
        assert_eq!(clean, "hello  world");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn extract_tokens_multiple_in_order() {
        let (clean, names) = extract_skill_tokens("$a then $b then $a");
        assert_eq!(clean, " then  then ");
        assert_eq!(names, vec!["a", "b", "a"]);
    }

    #[test]
    fn extract_tokens_adjacent() {
        let (clean, names) = extract_skill_tokens("x$a$b");
        assert_eq!(clean, "x");
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn extract_tokens_hyphenated_name() {
        let (clean, names) = extract_skill_tokens("$repo-memory");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["repo-memory"]);
    }

    #[test]
    fn extract_tokens_dollar_then_non_alpha_is_literal() {
        let (clean, names) = extract_skill_tokens("text $ more");
        assert_eq!(clean, "text $ more");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_name_terminates_at_non_name_char() {
        let (clean, names) = extract_skill_tokens("$skill followed by text");
        assert_eq!(clean, " followed by text");
        assert_eq!(names, vec!["skill"]);
    }

    #[test]
    fn extract_tokens_double_brace_not_a_token() {
        let (clean, names) = extract_skill_tokens("{{not a token}}");
        assert_eq!(clean, "{{not a token}}");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_dollar_uppercase_is_literal() {
        let (clean, names) = extract_skill_tokens("env $HOME path");
        assert_eq!(clean, "env $HOME path");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_double_dollar_is_literal() {
        let (clean, names) = extract_skill_tokens("cost is $$ total");
        assert_eq!(clean, "cost is $$ total");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_utf8_text_preserved() {
        let (clean, names) = extract_skill_tokens("$review héllo 日本語");
        assert_eq!(clean, " héllo 日本語");
        assert_eq!(names, vec!["review"]);
    }

    // ----- strip_resolved_skill_tokens tests -----

    #[test]
    fn strip_resolved_greedy_glued_name_preserved_verbatim() {
        // The greedy `[a-z0-9-]` charset scans `review1` as the *whole* token
        // name. Since `review1` != `review`, it is unresolved and the entire
        // `$review1` is kept verbatim — no content is lost. (Contrast with the
        // old `extract_skill_tokens`, which stripped `review1` unconditionally
        // and lost the `$review1` bytes.)
        let resolved: HashSet<String> = ["review"].iter().map(|s| s.to_string()).collect();
        assert_eq!(strip_resolved_skill_tokens("$review1) task", &resolved), "$review1) task");
    }

    #[test]
    fn strip_resolved_space_separated_resolved_drops_token() {
        // With a separating space, the scanner reads just `review` (resolving
        // it) and leaves the rest intact — the picker inserts a trailing
        // space precisely to enable this clean path.
        let resolved: HashSet<String> = ["review"].iter().map(|s| s.to_string()).collect();
        assert_eq!(strip_resolved_skill_tokens("$review 1) task", &resolved), " 1) task");
    }

    #[test]
    fn strip_resolved_keeps_unresolved_verbatim() {
        let resolved: HashSet<String> = HashSet::new();
        assert_eq!(strip_resolved_skill_tokens("$bogus text", &resolved), "$bogus text");
    }

    #[test]
    fn strip_resolved_mixed_tokens() {
        // Resolved `review` is dropped; unresolved `bogus` is preserved.
        let resolved: HashSet<String> = ["review"].iter().map(|s| s.to_string()).collect();
        assert_eq!(strip_resolved_skill_tokens("$review $bogus mixed", &resolved), " $bogus mixed");
    }

    #[test]
    fn strip_resolved_empty_input() {
        let resolved: HashSet<String> = HashSet::new();
        assert_eq!(strip_resolved_skill_tokens("", &resolved), "");
    }

    #[test]
    fn strip_resolved_literal_dollar_untouched() {
        // `$5` / `$HOME` / trailing `$` are never tokens, so they pass through
        // regardless of `resolved`.
        let resolved: HashSet<String> = ["x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(strip_resolved_skill_tokens("price is $5 $HOME total $", &resolved), "price is $5 $HOME total $");
    }

    #[test]
    fn strip_resolved_utf8_preserved() {
        let resolved: HashSet<String> = ["review"].iter().map(|s| s.to_string()).collect();
        assert_eq!(strip_resolved_skill_tokens("$review héllo 日本語", &resolved), " héllo 日本語");
    }


    // ------------------------------------------------------------------
    // Combined-content cases: skill token mixed with other input text.
    // These lock in the guarantee that `$name` is parsed correctly even
    // when surrounded by arbitrary user prose.
    // ------------------------------------------------------------------

    #[test]
    fn extract_tokens_token_at_end_after_text() {
        // Skill at the very end, after other content.
        let (clean, names) = extract_skill_tokens("do stuff $alpha");
        assert_eq!(clean, "do stuff ");
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    fn extract_tokens_realistic_combined_input() {
        // A realistic prompt: skill token + a natural-language task.
        let (clean, names) =
            extract_skill_tokens("$repo-memory Summarize the recent changes.");
        assert_eq!(clean, " Summarize the recent changes.");
        assert_eq!(names, vec!["repo-memory"]);
    }

    #[test]
    fn extract_tokens_curly_brace_in_other_content_preserved() {
        // Other content with `{...}` that is NOT a skill token must survive.
        let (clean, names) = extract_skill_tokens("use {x} then $skill now");
        assert_eq!(clean, "use {x} then  now");
        assert_eq!(names, vec!["skill"]);
    }

    #[test]
    fn extract_tokens_dollar_in_other_content_preserved() {
        // A lone `$` in the surrounding text is not a token delimiter.
        let (clean, names) = extract_skill_tokens("price is $5 $skill done");
        assert_eq!(clean, "price is $5  done");
        assert_eq!(names, vec!["skill"]);
    }

    #[test]
    fn extract_tokens_multiple_skills_split_by_text() {
        // Two skill tokens separated by substantial prose.
        let (clean, names) =
            extract_skill_tokens("$a first task then $b second task");
        assert_eq!(clean, " first task then  second task");
        assert_eq!(names, vec!["a", "b"]);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    // Tests share the process-global cache across threads, so every test uses
    // a fresh tempdir to guarantee fingerprints never collide.
    fn write(path: impl AsRef<Path>, contents: &str) {
        let p = path.as_ref();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    #[test]
    fn cache_serves_repeat_calls_and_invalidates_on_edit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("alpha.md"), "one");
        let first = discover_cached(dir.path());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "alpha");
        // Unchanged fingerprint must be served from the cache verbatim.
        let second = discover_cached(dir.path());
        assert_eq!(first, second);
        thread::sleep(Duration::from_millis(15));
        write(dir.path().join("alpha.md"), "---\nname: beta\n---\ntwo");
        let third = discover_cached(dir.path());
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].name, "beta", "mtime change must force a rescan");
    }

    #[test]
    fn cache_invalidates_on_file_add() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("alpha.md"), "one");
        assert_eq!(discover_cached(dir.path()).len(), 1);
        thread::sleep(Duration::from_millis(15));
        write(dir.path().join("second.md"), "two");
        assert_eq!(discover_cached(dir.path()).len(), 2);
    }

    #[test]
    fn distinct_roots_do_not_collide() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path().join("alpha.md"), "one");
        write(b.path().join("beta.md"), "two");
        // Alternate roots against the single-entry cache: each lookup must
        // key on the root and never serve the other directory's skills.
        let in_a = discover_cached(a.path());
        let in_b = discover_cached(b.path());
        let in_a_again = discover_cached(a.path());
        let in_b_again = discover_cached(b.path());
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].name, "alpha");
        assert_eq!(in_b.len(), 1);
        assert_eq!(in_b[0].name, "beta");
        assert_eq!(in_a_again, in_a);
        assert_eq!(in_b_again, in_b);
    }
}
