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

use std::path::{Path, PathBuf};

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

/// Built-in skills shipped with the binary and embedded at compile time via
/// [`include_str!`]. Each entry is `(skill_dir, &[(file_name, contents)])`.
/// Seeded into `~/.opencoder/skills` on first startup so a fresh install ships
/// the `do-and-done -> review -> submit` workflow plus `repo-local-memory`.
const BUILTIN_SKILLS: &[(&str, &[(&str, &str)])] = &[
    (
        "do-and-done",
        &[(
            "SKILL.md",
            include_str!("../assets/skills/do-and-done/SKILL.md"),
        )],
    ),
    (
        "repo-local-memory",
        &[
            (
                "SKILL.md",
                include_str!("../assets/skills/repo-local-memory/SKILL.md"),
            ),
            (
                "EXAMPLES.md",
                include_str!("../assets/skills/repo-local-memory/EXAMPLES.md"),
            ),
            (
                "TEMPLATES.md",
                include_str!("../assets/skills/repo-local-memory/TEMPLATES.md"),
            ),
        ],
    ),
    (
        "review",
        &[("SKILL.md", include_str!("../assets/skills/review/SKILL.md"))],
    ),
    (
        "submit",
        &[("SKILL.md", include_str!("../assets/skills/submit/SKILL.md"))],
    ),
];

/// Dependency-gated skills — hidden until the user runs
/// `install-skills-dep.sh` which creates a sentinel file in `skills_dir()`.
/// Seeded independently of [`BUILTIN_SKILLS`] so a fresh install does not get
/// these skills unless the optional deps (tmux, chromium) are installed.
const DEP_GATED_SKILLS: &[(&str, &[(&str, &str)])] = &[
    (
        "ssh-pty",
        &[(
            "SKILL.md",
            include_str!("../assets/skills/ssh-pty/SKILL.md"),
        )],
    ),
    (
        "chrome-headless",
        &[(
            "SKILL.md",
            include_str!("../assets/skills/chrome-headless/SKILL.md"),
        )],
    ),
];

/// Skill directory whose presence means "built-ins already seeded". Gating on
/// `review` means a user deleting any other skill won't trigger a full reseed,
/// but a truly fresh install (no `review` dir) gets the full default set.
const SEED_GATE: &str = "review";

/// Sentinel file (inside [`skills_dir`]) whose presence means the user ran
/// `install-skills-dep.sh` and the optional-dependency skills should be
/// seeded. Independent of `SEED_GATE`.
pub const DEPS_SENTINEL: &str = ".skills-deps";

/// Seed the built-in skills into `~/.opencoder/skills` if they are missing.
///
/// Idempotent and best-effort: if the gate directory (`review`) already exists
/// we assume the user has their own setup and touch nothing; otherwise we write
/// every shipped skill, skipping files that already exist so partial user edits
/// are never clobbered. Errors are logged via `tracing` and never propagated —
/// seeding must never block startup.
pub fn seed_builtin_skills() {
    let root = skills_dir();
    if root.join(SEED_GATE).exists() {
        return;
    }
    if let Err(e) = seed_builtin_skills_in(&root) {
        tracing::warn!(
            "failed to seed built-in skills into {}: {e}",
            root.display()
        );
    }
}

/// Filesystem-writing core, factored out so tests can target a tempdir.
///
/// Always writes every shipped skill (creating its directory), but never
/// overwrites a file that already exists. The gate check lives in the public
/// [`seed_builtin_skills`] entry point; this fn is the pure writer.
pub fn seed_builtin_skills_in(root: &Path) -> std::io::Result<()> {
    for (skill_dir, files) in BUILTIN_SKILLS {
        let dir = root.join(skill_dir);
        std::fs::create_dir_all(&dir)?;
        for (name, content) in *files {
            let path = dir.join(name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Seed the dependency-gated skills (ssh-pty, chrome-headless) into
/// `~/.opencode/skills` if the [`DEPS_SENTINEL`] file exists.
///
/// Independent of [`seed_builtin_skills`]: a fresh install gets only the
/// built-in skills until the user explicitly installs the optional deps via
/// `install-skills-dep.sh`. Idempotent and best-effort.
pub fn seed_dep_gated_skills() {
    let root = skills_dir();
    if !root.join(DEPS_SENTINEL).exists() {
        return;
    }
    if let Err(e) = seed_dep_gated_skills_in(&root) {
        tracing::warn!(
            "failed to seed dep-gated skills into {}: {e}",
            root.display()
        );
    }
}

/// Filesystem-writing core for dep-gated skills, factored out for tests.
/// Like [`seed_builtin_skills_in`] but writes the dep-gated set; never
/// overwrites existing files. Sentinel-gated: writes nothing unless
/// [`DEPS_SENTINEL`] exists under `root`, mirroring the gate in
/// [`seed_dep_gated_skills`] so the contract is testable against a tempdir.
pub fn seed_dep_gated_skills_in(root: &Path) -> std::io::Result<()> {
    if !root.join(DEPS_SENTINEL).exists() {
        return Ok(());
    }
    for (skill_dir, files) in DEP_GATED_SKILLS {
        let dir = root.join(skill_dir);
        std::fs::create_dir_all(&dir)?;
        for (name, content) in *files {
            let path = dir.join(name);
            if path.exists() {
                continue;
            }
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Write `install-skills-dep.sh` into `~/.opencode/` so the user can discover
/// and run it. Idempotent: skips if the file already exists.
pub fn write_install_script() {
    let dir = match dirs::home_dir() {
        Some(h) => h.join(".opencode"),
        None => return,
    };
    if let Err(e) = write_install_script_in(&dir) {
        tracing::warn!("failed to write install script to {}: {e}", dir.display());
    }
}

/// Filesystem-writing core for the install script, factored out so tests can
/// target a tempdir. Idempotent: skips if the file already exists. Sets
/// executable permissions on Unix.
pub fn write_install_script_in(base: &Path) -> std::io::Result<()> {
    let path = base.join("install-skills-dep.sh");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, INSTALL_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, PermissionsExt::from_mode(0o755))?;
    }
    Ok(())
}

/// Embedded copy of `scripts/install-skills-dep.sh`, written to
/// `~/.opencode/install-skills-dep.sh` on startup so users can discover the
/// optional-dependency installer.
const INSTALL_SCRIPT: &str = include_str!("../../../scripts/install-skills-dep.sh");

/// Scan `~/.opencoder/skills` and return every skill found, sorted by name.
///
/// A missing or unreadable directory is not an error — it yields an empty
/// `Vec`, so the TUI picker simply reports "no skills" instead of crashing.
pub fn discover() -> Vec<Skill> {
    discover_in(&skills_dir())
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

/// Strip every `{$name}` token from `text`, returning the cleaned text and the
/// list of skill names in the order they appeared (empty names from `{$}` are
/// skipped; duplicates are preserved here and deduped by the caller).
///
/// An unclosed `{$abc` (no matching `}`) is treated as literal text. The scan
/// is UTF-8 safe: `{$` are ASCII so byte-level detection never splits a
/// multi-byte char.
pub fn extract_skill_tokens(text: &str) -> (String, Vec<String>) {
    let mut clean = String::with_capacity(text.len());
    let mut names = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'{' && i + 1 < text.len() && bytes[i + 1] == b'$' {
            let after = i + 2;
            if let Some(rel) = text[after..].find('}') {
                let close = after + rel;
                let name = text[after..close].trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
                i = close + 1;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        clean.push(ch);
        i += ch.len_utf8();
    }
    (clean, names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        let (clean, names) = extract_skill_tokens("{$code}");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn extract_tokens_mid_text_preserves_surrounding_text() {
        let (clean, names) = extract_skill_tokens("hello {$code} world");
        assert_eq!(clean, "hello  world");
        assert_eq!(names, vec!["code"]);
    }

    #[test]
    fn extract_tokens_multiple_in_order() {
        let (clean, names) = extract_skill_tokens("{$a} then {$b} then {$a}");
        assert_eq!(clean, " then  then ");
        assert_eq!(names, vec!["a", "b", "a"]);
    }

    #[test]
    fn extract_tokens_adjacent() {
        let (clean, names) = extract_skill_tokens("x{$a}{$b}y");
        assert_eq!(clean, "xy");
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn extract_tokens_name_with_spaces_trimmed() {
        let (clean, names) = extract_skill_tokens("{$  spaced  }");
        assert_eq!(clean, "");
        assert_eq!(names, vec!["spaced"]);
    }

    #[test]
    fn extract_tokens_empty_name_skipped() {
        let (clean, names) = extract_skill_tokens("text {$} more");
        assert_eq!(clean, "text  more");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_unclosed_is_literal() {
        let (clean, names) = extract_skill_tokens("{$unclosed followed by text");
        assert_eq!(clean, "{$unclosed followed by text");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_double_brace_not_a_token() {
        let (clean, names) = extract_skill_tokens("{{not a token}}");
        assert_eq!(clean, "{{not a token}}");
        assert!(names.is_empty());
    }

    #[test]
    fn extract_tokens_utf8_text_preserved() {
        let (clean, names) = extract_skill_tokens("héllo {$wörld} 日本語");
        assert_eq!(clean, "héllo  日本語");
        assert_eq!(names, vec!["wörld"]);
    }

    // ----- write_install_script_in tests -----

    #[test]
    fn write_install_script_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_install_script_in(base).unwrap();
        let script = base.join("install-skills-dep.sh");
        assert!(script.is_file());
        let content = std::fs::read_to_string(&script).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn write_install_script_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_install_script_in(base).unwrap();
        // Write a sentinel to detect overwrite.
        let script = base.join("install-skills-dep.sh");
        std::fs::write(&script, "SENTINEL").unwrap();
        write_install_script_in(base).unwrap();
        let content = std::fs::read_to_string(&script).unwrap();
        assert_eq!(content, "SENTINEL");
    }
}
