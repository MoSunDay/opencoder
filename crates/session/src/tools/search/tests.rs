use super::*;
use opencoder_core::Tool;
use serde_json::json;
use std::io::Write;

fn ctx_for(dir: &tempfile::TempDir) -> ToolContext {
    ToolContext {
        extra_env: Vec::new(),
        session_id: "test".into(),
        message_id: "test".into(),
        agent: "explore".into(),
        working_dir: dir.path().to_path_buf(),
        max_output: 4096,
        proxy: None,
        tools_path: None,
    }
}

#[tokio::test]
async fn search_finds_matching_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("greet.rs")).unwrap();
    writeln!(f, "fn main() {{").unwrap();
    writeln!(f, "    println!(\"hello world\");").unwrap();
    writeln!(f, "}}").unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tool
        .execute(json!({ "pattern": "hello world" }), &ctx)
        .await
        .unwrap();
    assert!(!out.is_error, "expected success, got: {}", out.content);
    // Match line is line 2 in `greet.rs`; expect `greet.rs:2: ...hello world...`.
    assert!(
        out.content.contains("greet.rs:2:"),
        "expected match path:line marker, got: {}",
        out.content
    );
    assert!(
        out.content.contains("hello world"),
        "expected match content, got: {}",
        out.content
    );
}

#[tokio::test]
async fn search_no_matches_returns_ok() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
    writeln!(f, "alpha").unwrap();
    writeln!(f, "beta").unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tool
        .execute(json!({ "pattern": "this_pattern_does_not_exist" }), &ctx)
        .await
        .unwrap();
    assert!(!out.is_error, "no matches is not an error");
    assert_eq!(out.content, "no matches");
}

#[tokio::test]
async fn search_empty_pattern_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tool.execute(json!({ "pattern": "" }), &ctx).await.unwrap();
    assert!(out.is_error, "empty pattern must be an error");
    assert!(out.content.contains("non-empty"));
}

/// Procfs-style blowup: a chain of distinct directories, each reached by
/// two sibling symlinks, ending in a non-matching file. Every hop is a
/// *different* directory, so ancestor-chain loop detection never fires and
/// there is no match cap to short-circuit; without the re-entry guard the
/// walker explores 2^N paths (2^25 here) and pins a core. With the guard
/// it must finish in milliseconds and report no matches.
#[tokio::test]
async fn search_terminates_on_distinct_hop_link_fanout() {
    let dir = tempfile::tempdir().unwrap();
    let mut prev = dir.path().join("lvl0");
    std::fs::create_dir(&prev).unwrap();
    for i in 1..=25u32 {
        let next = dir.path().join(format!("lvl{i}"));
        std::fs::create_dir(&next).unwrap();
        std::os::unix::fs::symlink(&next, prev.join("x")).unwrap();
        std::os::unix::fs::symlink(&next, prev.join("y")).unwrap();
        prev = next;
    }
    writeln!(
        std::fs::File::create(prev.join("end.txt")).unwrap(),
        "unrelated"
    )
    .unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tool.execute(json!({ "pattern": "never_matches_anything" }), &ctx),
    )
    .await
    .expect("search must terminate on link fan-out")
    .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(out.content, "no matches");
}

/// A plain a->b->a symlink cycle must terminate and still find matches.
#[tokio::test]
async fn search_terminates_on_symlink_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut f = std::fs::File::create(a.join("f.txt")).unwrap();
    writeln!(f, "cycle needle").unwrap();
    std::os::unix::fs::symlink(&b, a.join("to_b")).unwrap();
    std::os::unix::fs::symlink(&a, b.join("to_a")).unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tool.execute(json!({ "pattern": "cycle needle" }), &ctx),
    )
    .await
    .expect("search must terminate on symlink cycle")
    .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("cycle needle"), "{}", out.content);
}

/// Several sibling links into the same physical directory: links are
/// followed, but the target must be searched exactly once.
#[tokio::test]
async fn search_no_dir_reentry_via_sibling_links() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let mut f = std::fs::File::create(real.join("needle.txt")).unwrap();
    writeln!(f, "golden needle").unwrap();
    for l in ["l1", "l2", "l3"] {
        std::os::unix::fs::symlink(&real, dir.path().join(l)).unwrap();
    }
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tool
        .execute(json!({ "pattern": "golden needle" }), &ctx)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    let hits = out
        .content
        .lines()
        .filter(|l| l.contains("golden needle"))
        .count();
    assert_eq!(
        hits, 1,
        "a physical dir must be searched exactly once, got: {}",
        out.content
    );
}

/// Following is preserved: a symlinked directory passed as `path` still
/// gets searched.
#[tokio::test]
async fn search_follows_symlinked_base_dir() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let mut f = std::fs::File::create(real.join("greet.txt")).unwrap();
    writeln!(f, "via link").unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join("lnk")).unwrap();
    let ctx = ctx_for(&dir);
    let tool = SearchTool;
    let out = tool
        .execute(json!({ "pattern": "via link", "path": "lnk" }), &ctx)
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("via link"), "{}", out.content);
}
