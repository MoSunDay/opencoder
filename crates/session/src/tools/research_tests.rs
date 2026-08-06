use super::*;

#[test]
fn serp_url_builds_per_engine_urls() {
    let q = "Rust 性能";
    assert_eq!(
        serp_url("bing", q).as_str(),
        "https://cn.bing.com/search?q=Rust+%E6%80%A7%E8%83%BD"
    );
    assert_eq!(
        serp_url("baidu", q).as_str(),
        "https://www.baidu.com/s?wd=Rust+%E6%80%A7%E8%83%BD"
    );
    assert_eq!(
        serp_url("sogou", q).as_str(),
        "https://www.sogou.com/web?query=Rust+%E6%80%A7%E8%83%BD"
    );
    assert_eq!(
        serp_url("ddg", q).as_str(),
        "https://html.duckduckgo.com/html/?q=Rust+%E6%80%A7%E8%83%BD"
    );
    // google
    assert_eq!(
        serp_url("google", q).as_str(),
        "https://www.google.com/search?q=Rust+%E6%80%A7%E8%83%BD&hl=en&num=10&nfpr=1"
    );
    // github
    assert_eq!(
        serp_url("github", q).as_str(),
        "https://github.com/search?q=Rust+%E6%80%A7%E8%83%BD&type=repositories"
    );
    // hf / huggingface
    assert_eq!(
        serp_url("hf", q).as_str(),
        "https://huggingface.co/models?search=Rust+%E6%80%A7%E8%83%BD"
    );
    assert_eq!(
        serp_url("huggingface", q).as_str(),
        "https://huggingface.co/models?search=Rust+%E6%80%A7%E8%83%BD"
    );
    // unknown engine falls back to bing
    assert_eq!(
        serp_url("unknownengine", q).as_str(),
        "https://cn.bing.com/search?q=Rust+%E6%80%A7%E8%83%BD"
    );
}

fn res(title: &str, url: &str) -> SearchResult {
    SearchResult {
        title: title.into(),
        url: url.into(),
        snippet: String::new(),
    }
}

#[test]
fn merge_results_dedups_by_title_and_host() {
    let per = vec![
        vec![
            res("Rust Lang", "https://www.rust-lang.org/"),
            res("Rust Lang", "https://rust-lang.org/zh-CN"),
        ],
        vec![
            res("rust lang", "https://www.rust-lang.org/zh-CN"),
            res("Docs", "https://doc.rust-lang.org/"),
        ],
    ];
    let merged = merge_results(per, 10);
    assert_eq!(
        merged.len(),
        3,
        "cross-engine dup (same host+title) collapses"
    );
    assert_eq!(
        merged[0].url, "https://www.rust-lang.org/",
        "earlier engine wins"
    );
    assert_eq!(merged[2].url, "https://doc.rust-lang.org/");
}

#[test]
fn merge_results_respects_limit() {
    let per = vec![vec![
        res("a", "https://a.com"),
        res("b", "https://b.com"),
        res("c", "https://c.com"),
    ]];
    assert_eq!(merge_results(per, 2).len(), 2);
}

#[test]
fn slugify_produces_filesystem_safe_names() {
    assert_eq!(slugify("Rust programming 2026!"), "rust-programming-2026");
    assert_eq!(slugify("  spaced  "), "spaced");
    assert_eq!(slugify("如何评价"), "", "fully-CJK collapses to empty");
}

#[test]
fn build_report_renders_sources_and_links() {
    let a = web_extract::ExtractedArticle {
        title: "标题".into(),
        author: "作者".into(),
        date: "".into(),
        url: "https://real.example/post".into(),
        content: "正文节选。".into(),
    };
    let report = build_report(
        "查询",
        &["bing".into()],
        &[a],
        &[res("L", "https://l.com")],
        8000,
    );
    assert!(report.starts_with("# Research: 查询"));
    assert!(report.contains("Engines: bing"));
    assert!(report.contains("## 1. 标题"));
    assert!(report.contains("- url: https://real.example/post"));
    assert!(report.contains("正文节选。"));
    assert!(report.contains("## Raw search results"));
    assert!(report.contains("L — https://l.com"));
}

#[test]
fn build_report_truncates_excerpt() {
    let a = web_extract::ExtractedArticle {
        title: "t".into(),
        author: String::new(),
        date: String::new(),
        url: "https://x.com".into(),
        content: "一二三四五六七八九十".into(),
    };
    let report = build_report("q", &["bing".into()], &[a], &[], 4);
    assert!(report.contains("一二三四"));
    assert!(!report.contains("五六七八"));
    assert!(report.contains("…"));
}

/// End-to-end smoke: bing SERP → first wikipedia result → article
/// extraction. Requires a real Chrome/Chromium binary and network.
#[tokio::test]
#[ignore = "requires real Chrome and network; run manually"]
async fn research_smoke_bing_wikipedia() {
    let serp = serp_url("bing", "Rust programming language");
    let html = chrome_headless::dump_dom(serp.as_str(), Some(4000), None, None)
        .await
        .unwrap();
    let results = serp::parse_search_results(&serp, &html, 8);
    let wiki = results
        .iter()
        .find(|r| r.url.contains("wikipedia.org"))
        .expect("expected a wikipedia result in the bing SERP");
    let page = chrome_headless::dump_dom(&wiki.url, Some(4000), None, None)
        .await
        .unwrap();
    let final_url = chrome_headless::extract_final_url(&page, &wiki.url);
    let a = web_extract::extract_article(&page, &Url::parse(&final_url).unwrap());
    assert!(!a.title.is_empty(), "title must not be empty");
    assert!(a.content.chars().count() > 200, "content too short");
}

/// rules/03 exemption: `write_report` is a pure function of `ToolContext`
/// + strings whose only side effect is a hermetic tempfs write under
/// `std::env::temp_dir()` (PID-scoped, no network, no shared state, and
/// removed afterwards). Moving it to `crates/session/tests/` would force
/// `write_report` to become `pub`, leaking internals for one test.
#[tokio::test]
async fn write_report_creates_markdown() {
    use opencoder_core::ToolContext;
    let dir = std::env::temp_dir().join(format!("oc-research-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ctx = ToolContext {
        session_id: "s".into(),
        message_id: "m".into(),
        agent: "act".into(),
        working_dir: dir.clone(),
        max_output: 4096,
        proxy: None,
    };
    let path = write_report(&ctx, "标题", "## 标题\n内容").unwrap();
    assert!(
        path.extension().is_some_and(|e| e == "md"),
        "{}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("## 标题"));
    assert!(body.contains("内容"));
    std::fs::remove_dir_all(&dir).ok();
}
