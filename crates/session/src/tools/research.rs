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
#[path = "research_tests.rs"]
mod tests;
