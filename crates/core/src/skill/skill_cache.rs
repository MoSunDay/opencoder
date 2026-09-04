//! Process-level skill discovery cache, split out of `skill.rs` to respect
//! its file line budget.
//!
//! [`discover_cached`] is the hot-path multi-root variant of discovery (the
//! UI submit path calls it per keypress): a hit costs one `read_dir` plus N
//! `stat` calls per root and never reads a skill file. The cache is keyed on
//! the full ordered root list — order decides first-wins shadowing, so a
//! reordered list is a different key — and invalidated by a combined
//! (path, mtime) fingerprint: the sorted, deduplicated union of the per-root
//! [`fingerprint`]s. Any change in ANY root's watched file set forces a
//! rescan; a root that does not exist fingerprints as absent, so a root
//! appearing or disappearing flips the fingerprint too. The scan core
//! (`discover_in`), the merge policy (`discover_all`) and the production
//! root assembly (`production_skill_roots`) stay in `skill.rs`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::{discover_all, Skill};

/// Cache entry: the combined (path, mtime) fingerprint captured at
/// discovery time plus the discovered skills behind an Arc so hits clone
/// cheaply.
struct DiscoverCacheEntry {
    files: Vec<(PathBuf, SystemTime)>,
    skills: Arc<Vec<Skill>>,
}

/// Process-level discovery cache, keyed on the single most recently
/// scanned ordered root list (same single-entry/evict-on-miss philosophy
/// as before the multi-root extension: the hot path touches one root list,
/// so one slot suffices). [`super::discover_in`] stays uncached so tests
/// pointing at tempdirs always observe the real directory.
static DISCOVER_CACHE: Mutex<Option<(Vec<PathBuf>, DiscoverCacheEntry)>> = Mutex::new(None);

/// Cached multi-root discovery for hot paths: returns the previously
/// discovered skills when the root list matches the cached one — same
/// roots, same order — and the combined [`fingerprint_all`] is unchanged,
/// otherwise rescans via [`super::discover_all`] and refreshes the cache.
/// The lock is never held across the rescan itself, so concurrent callers
/// never serialize on file I/O.
pub fn discover_cached(roots: &[PathBuf]) -> Vec<Skill> {
    let files = fingerprint_all(roots);
    {
        let cache = DISCOVER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_roots, entry)) = cache.as_ref() {
            if cached_roots.as_slice() == roots && entry.files == files {
                return entry.skills.as_ref().clone();
            }
        }
    }
    let skills = discover_all(roots);
    let mut cache = DISCOVER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *cache = Some((
        roots.to_vec(),
        DiscoverCacheEntry {
            files,
            skills: Arc::new(skills.clone()),
        },
    ));
    skills
}

/// Build the (path, mtime) fingerprint of exactly the file set
/// [`super::discover_in`] reads in one root: top-level `*.md` files plus
/// `<dir>/SKILL.md` for each subdirectory that has one. Sorted for stable
/// comparison. An empty result (unreadable/missing root or any stat
/// failure) is how an absent root fingerprints.
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

/// Combined fingerprint of a root list: the sorted, deduplicated union of
/// the per-root [`fingerprint`]s (a root listed twice, or two roots whose
/// watched file sets overlap, must not double-count). Any (path, mtime)
/// change in ANY watched file set makes this mismatch the cached copy and
/// forces a rescan.
fn fingerprint_all(roots: &[PathBuf]) -> Vec<(PathBuf, SystemTime)> {
    let mut out: Vec<(PathBuf, SystemTime)> =
        roots.iter().flat_map(|root| fingerprint(root)).collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::discover_all;
    use std::thread;
    use std::time::Duration;

    // Tests share the process-global cache across threads, so every test
    // uses fresh tempdirs to guarantee fingerprints never collide.

    fn write(path: impl AsRef<Path>, contents: &str) {
        let p = path.as_ref();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    /// Wrap one or more roots into the ordered list `discover_cached` takes.
    fn roots(dirs: &[&Path]) -> Vec<PathBuf> {
        dirs.iter().map(|p| p.to_path_buf()).collect()
    }

    #[test]
    fn cache_serves_repeat_calls_and_invalidates_on_edit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("alpha.md"), "one");
        let list = roots(&[dir.path()]);
        let first = discover_cached(&list);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "alpha");
        // Unchanged fingerprint must be served from the cache verbatim.
        let second = discover_cached(&list);
        assert_eq!(first, second);
        thread::sleep(Duration::from_millis(15));
        write(dir.path().join("alpha.md"), "---\nname: beta\n---\ntwo");
        let third = discover_cached(&list);
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].name, "beta", "mtime change must force a rescan");
    }

    #[test]
    fn cache_invalidates_on_file_add() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path().join("alpha.md"), "one");
        assert_eq!(discover_cached(&roots(&[dir.path()])).len(), 1);
        thread::sleep(Duration::from_millis(15));
        write(dir.path().join("second.md"), "two");
        assert_eq!(discover_cached(&roots(&[dir.path()])).len(), 2);
    }

    #[test]
    fn distinct_roots_do_not_collide() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path().join("alpha.md"), "one");
        write(b.path().join("beta.md"), "two");
        // Alternate root lists against the single-entry cache: each lookup
        // must key on the root list and never serve the other's skills.
        let in_a = discover_cached(&roots(&[a.path()]));
        let in_b = discover_cached(&roots(&[b.path()]));
        let in_a_again = discover_cached(&roots(&[a.path()]));
        let in_b_again = discover_cached(&roots(&[b.path()]));
        assert_eq!(in_a.len(), 1);
        assert_eq!(in_a[0].name, "alpha");
        assert_eq!(in_b.len(), 1);
        assert_eq!(in_b[0].name, "beta");
        assert_eq!(in_a_again, in_a);
        assert_eq!(in_b_again, in_b);
    }

    #[test]
    fn multi_root_cache_hits_then_rescans_on_second_root_edit() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path().join("alpha.md"), "one");
        write(b.path().join("beta.md"), "body-one");
        let list = roots(&[a.path(), b.path()]);
        let first = discover_cached(&list);
        assert_eq!(first.len(), 2);
        // Unchanged fingerprints in BOTH roots: served from the cache.
        assert_eq!(first, discover_cached(&list));
        // Touch a file in the SECOND root only: the combined fingerprint
        // must flip and force a rescan that observes the new content while
        // the untouched first root's skills survive.
        thread::sleep(Duration::from_millis(15));
        write(b.path().join("beta.md"), "---\nname: beta2\n---\nbody-two");
        let third = discover_cached(&list);
        assert_eq!(third.len(), 2, "alpha must survive the rescan");
        let beta = third.iter().find(|s| s.name != "alpha").unwrap();
        assert_eq!(beta.name, "beta2");
        assert_eq!(beta.body, "body-two");
    }

    #[test]
    fn shadowing_survives_cache_round_trip() {
        let agent = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        write(
            agent.path().join("shared.md"),
            "---\nname: shared\ndescription: agent\n---\nagent-body",
        );
        write(
            global.path().join("shared.md"),
            "---\nname: shared\ndescription: global\n---\nphantom",
        );
        let list = roots(&[agent.path(), global.path()]);
        let first = discover_cached(&list);
        assert_eq!(first.len(), 1, "same-name skill must dedupe to one entry");
        assert_eq!(first[0].description, "agent");
        // Second call must come from the cache AND still be the shadowing
        // version — a naive merge on rescan-only would be fine, but the
        // cached entry must never resurrect the shadowed global copy.
        let second = discover_cached(&list);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].description, "agent");
        assert_eq!(first, second);
    }

    #[test]
    fn missing_root_fingerprints_as_absent_until_it_appears() {
        let a = tempfile::tempdir().unwrap();
        write(a.path().join("alpha.md"), "one");
        let holder = tempfile::tempdir().unwrap();
        let later = holder.path().join("pool");
        let list = roots(&[a.path(), &later]);
        let first = discover_cached(&list);
        assert_eq!(first.len(), 1, "missing root must contribute nothing");
        assert_eq!(first, discover_cached(&list));
        thread::sleep(Duration::from_millis(15));
        write(later.join("beta.md"), "two"); // the root now exists
        assert_eq!(
            discover_cached(&list).len(),
            2,
            "a root appearing must flip the combined fingerprint"
        );
    }

    #[test]
    fn root_order_is_part_of_the_cache_key() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(
            a.path().join("shared.md"),
            "---\nname: shared\ndescription: agent\n---\nagent-body",
        );
        write(
            b.path().join("shared.md"),
            "---\nname: shared\ndescription: global\n---\nphantom",
        );
        // Identical watched file sets, different order: swapping the list
        // must not serve the previous entry — order decides shadowing.
        let ab = discover_cached(&roots(&[a.path(), b.path()]));
        let ba = discover_cached(&roots(&[b.path(), a.path()]));
        assert_eq!(ab.len(), 1);
        assert_eq!(ba.len(), 1);
        assert_eq!(ab[0].description, "agent");
        assert_eq!(ba[0].description, "global");
    }
    // ----- discover_all merge: first-wins shadowing (the policy this
    // ----- cache protects) -----

    #[test]
    fn discover_all_merges_disjoint_roots_sorted() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path().join("zeta.md"), "z");
        write(a.path().join("alpha.md"), "a");
        write(b.path().join("mid.md"), "m");
        write(b.path().join("beta.md"), "b");
        let found = discover_all(&[a.path().to_path_buf(), b.path().to_path_buf()]);
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "mid", "zeta"]);
    }

    #[test]
    fn discover_all_first_root_shadows_same_name() {
        let agent = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        write(
            agent.path().join("shared").join("SKILL.md"),
            "---\nname: shared\ndescription: agent-private version\n---\nagent body",
        );
        write(
            global.path().join("shared").join("SKILL.md"),
            "---\nname: shared\ndescription: global version\n---\nphantom body",
        );
        write(global.path().join("only-global.md"), "global only");
        let found = discover_all(&[agent.path().to_path_buf(), global.path().to_path_buf()]);
        assert_eq!(found.len(), 2, "shadowed copy must be dropped, not merged");
        let shared = found.iter().find(|s| s.name == "shared").unwrap();
        assert_eq!(shared.description, "agent-private version");
        assert_eq!(shared.body, "agent body");
        assert_eq!(
            shared.source,
            agent.path().join("shared").join("SKILL.md"),
            "the earlier (agent) root's file must win"
        );
    }

    #[test]
    fn discover_all_missing_root_contributes_nothing() {
        let a = tempfile::tempdir().unwrap();
        let gone = a.path().join("no-such-root");
        write(a.path().join("solo.md"), "one");
        let found = discover_all(&[a.path().to_path_buf(), gone]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "solo");
    }

}
