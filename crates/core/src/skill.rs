//! User-authored "skill" instruction packs.
//!
//! A skill is a markdown file whose body is injected into the agent's system
//! prompt when the user activates it from the TUI (`$` menu). This lets users
//! drop reusable operating procedures (a SKILL.md per topic) into
//! `~/.opencoder/skills/` and load them on demand without touching the agent
//! registry or config.
//!
//! Two on-disk layouts are accepted, mirroring the opencode skill convention:
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
/// (`~/.opencoder/skills`). Returns `~/.opencode/skills` only as an absolute
/// fallback when no home directory can be resolved, so discovery never panics.
pub fn skills_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".opencoder").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".opencode").join("skills"))
}

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
}
