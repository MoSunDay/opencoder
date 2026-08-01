//! Deep-research pipeline tool: multi-engine search → merge/dedup → render
//! each result with headless Chrome → site-aware article extraction →
//! markdown report written under `<working_dir>/.research/`. The pure helpers
//! (`serp_url` / `merge_results` / `slugify` / `build_report`) are unit-tested
//! in the default build; the tool body reuses `chrome_headless::dump_dom` and
//! `web_extract::extract_article`, so a single `research` call replaces the
//! whole search→fetch→extract chain.

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use url::Url;

use super::web_read::SearchResult;
use super::{chrome_headless, web_extract, web_read};

/// Build the SERP URL for `engine` (bing / baidu / sogou / ddg). Unknown
/// engines fall back to Bing. The query is form-encoded for its key.
pub fn serp_url(engine: &str, query: &str) -> Url {
    let enc = |key: &str| {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair(key, query)
            .finish()
    };
    let raw = match engine {
        "baidu" => format!("https://www.baidu.com/s?{}", enc("wd")),
        "sogou" => format!("https://www.sogou.com/web?{}", enc("query")),
        "ddg" => format!("https://html.duckduckgo.com/html/?{}", enc("q")),
        _ => format!("https://cn.bing.com/search?{}", enc("q")),
    };
    Url::parse(&raw).expect("engine SERP URLs are static and always parseable")
}

/// Merge per-engine result lists into one deduped list, preserving engine
/// priority (earlier engines win a duplicate). Dedup key = lowercase title +
/// URL host, which tolerates engines that paraphrase the same page.
pub fn merge_results(per_engine: Vec<Vec<SearchResult>>, limit: usize) -> Vec<SearchResult> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for list in per_engine {
        for r in list {
            if out.len() >= limit {
                return out;
            }
            let host = Url::parse(&r.url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .unwrap_or_default();
            if seen.insert(format!("{}|{host}", r.title.to_lowercase())) {
                out.push(r);
            }
        }
    }
    out
}

/// Filesystem-safe slug: lowercase, non-alphanumeric runs collapse to `-`,
/// leading/trailing dashes trimmed. Pure-ASCII queries only keep ASCII; a
/// fully-CJK query collapses to empty (caller falls back).
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Compose the final markdown report: header (query + engines + source count),
/// one `##` section per source with title / real URL / optional metadata /
/// content excerpt, then the raw merged result list for reference.
pub fn build_report(
    query: &str,
    engines: &[String],
    sources: &[web_extract::ExtractedArticle],
    links: &[SearchResult],
    per_page_chars: usize,
) -> String {
    let mut out = format!("# Research: {query}\n\n");
    out.push_str(&format!("Engines: {}\n\n", engines.join(", ")));
    out.push_str(&format!("Sources: {}\n\n", sources.len()));
    for (i, a) in sources.iter().enumerate() {
        let title = if a.title.is_empty() { &a.url } else { &a.title };
        out.push_str(&format!("## {}. {title}\n", i + 1));
        out.push_str(&format!("- url: {}\n", a.url));
        if !a.author.is_empty() {
            out.push_str(&format!("- author: {}\n", a.author));
        }
        if !a.date.is_empty() {
            out.push_str(&format!("- date: {}\n", a.date));
        }
        out.push('\n');
        let excerpt: String = a.content.chars().take(per_page_chars).collect();
        out.push_str(&excerpt);
        if a.content.chars().count() > per_page_chars {
            out.push_str("\n…");
        }
        out.push_str("\n\n");
    }
    if !links.is_empty() {
        out.push_str("## Raw search results\n\n");
        for (i, r) in links.iter().enumerate() {
            out.push_str(&format!(
                "{}. {} — {} ({})\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }
    }
    out
}

/// Persist the report under `<working_dir>/.research/<slug>-<ts>.md`, keeping
/// all writes inside the working directory.
fn write_report(
    ctx: &ToolContext,
    query: &str,
    report: &str,
) -> std::io::Result<std::path::PathBuf> {
    let dir = ctx.working_dir.join(".research");
    std::fs::create_dir_all(&dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let slug = slugify(query);
    let slug = if slug.is_empty() {
        "research".to_string()
    } else {
        slug
    };
    let path = dir.join(format!("{slug}-{ts}.md"));
    std::fs::write(&path, report)?;
    Ok(path)
}

pub struct ResearchTool;

#[async_trait]
impl Tool for ResearchTool {
    fn name(&self) -> &str {
        "research"
    }
    fn description(&self) -> &str {
        "Multi-engine deep research: renders Bing/Baidu/Sogou/DDG SERPs with \
         headless Chrome, merges + dedups results, renders each result page and \
         extracts structured articles (site-aware profiles), then writes a \
         markdown report to <working_dir>/.research/ and returns the path plus \
         per-source summaries. Engines that return nothing are skipped."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("query".into(), json::prop_str("The research question."));
        props.insert(
            "max_results".into(),
            serde_json::json!({"type": "integer", "description": "Max sources to fetch (1-10, default 6)."}),
        );
        props.insert(
            "engines".into(),
            serde_json::json!({
                "type": "array",
                "items": {"type": "string", "enum": ["bing", "baidu", "sogou", "ddg"]},
                "description": "Engines to query (default [\"bing\", \"baidu\"])."
            }),
        );
        props.insert(
            "per_page_chars".into(),
            serde_json::json!({"type": "integer", "description": "Max chars of each source excerpt in the report (default 8000)."}),
        );
        json::object_schema(Value::Object(props), &["query"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return Ok(ToolOutput::err("query is required"));
        }
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(6)
            .clamp(1, 10) as usize;
        let per_page_chars = input
            .get("per_page_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(8000)
            .clamp(500, 20_000) as usize;
        let engines: Vec<String> = input
            .get("engines")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| vec!["bing".into(), "baidu".into()]);
        if engines.is_empty() {
            return Ok(ToolOutput::err("engines must not be empty"));
        }

        // 1. Render each engine SERP; engines that fail or parse empty are
        //    skipped so one anti-bot wall never kills the whole pipeline.
        let mut per_engine = Vec::new();
        for engine in &engines {
            let serp = serp_url(engine, &query);
            match chrome_headless::dump_dom(serp.as_str(), Some(3000), None).await {
                Ok(html) => {
                    let results = web_read::parse_search_results(&serp, &html, 10);
                    if results.is_empty() {
                        tracing::warn!(
                            "research: engine {engine} returned no parseable results, skipping"
                        );
                    }
                    per_engine.push(results);
                }
                Err(e) => tracing::warn!("research: engine {engine} failed: {e}; skipping"),
            }
        }
        let merged = merge_results(per_engine, max_results);
        if merged.is_empty() {
            return Ok(ToolOutput::err(
                "no search results from any engine (anti-bot wall or network issue); try different engines",
            ));
        }

        // 2. Render each result and extract a structured article (unwinding
        //    redirect links so the report carries real URLs).
        let mut sources = Vec::new();
        let mut failures = 0usize;
        for r in &merged {
            match chrome_headless::dump_dom(&r.url, Some(3000), None).await {
                Ok(html) => {
                    let final_url = chrome_headless::extract_final_url(&html, &r.url);
                    let u = Url::parse(&final_url).unwrap_or_else(|_| Url::parse(&r.url).unwrap());
                    sources.push(web_extract::extract_article(&html, &u));
                }
                Err(e) => {
                    failures += 1;
                    tracing::warn!("research: fetch {} failed: {e}", r.url);
                }
            }
        }

        let report = build_report(&query, &engines, &sources, &merged, per_page_chars);
        let path = match write_report(ctx, &query, &report) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::err(format!("failed to write report: {e}"))),
        };

        // 3. Concise per-source summary for the model; the full report is on disk.
        let mut body = format!("Research report written to: {}\n\n", path.display());
        body.push_str(&format!(
            "Sources fetched: {} ({} failed)\n\n",
            sources.len(),
            failures
        ));
        for a in &sources {
            let title = if a.title.is_empty() {
                "(untitled)"
            } else {
                &a.title
            };
            body.push_str(&format!("{title} — {}\n", a.url));
        }
        Ok(truncate_output(body, ctx.max_output))
    }
}

#[cfg(test)]
mod tests {
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
        // unknown engine falls back to bing
        assert_eq!(
            serp_url("google", q).as_str(),
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
        let html = chrome_headless::dump_dom(serp.as_str(), Some(4000), None)
            .await
            .unwrap();
        let results = web_read::parse_search_results(&serp, &html, 8);
        let wiki = results
            .iter()
            .find(|r| r.url.contains("wikipedia.org"))
            .expect("expected a wikipedia result in the bing SERP");
        let page = chrome_headless::dump_dom(&wiki.url, Some(4000), None)
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
}
