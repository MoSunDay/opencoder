//! Multi-engine web search, gated behind the `browser` cargo feature. Loads
//! the SERP page (Bing / Baidu / Sogou / DuckDuckGo) through obscura (so it
//! survives anti-bot walls), then parses `{title, url, snippet}` rows via
//! [`super::web_read::parse_search_results`], which dispatches by URL host.
//!
//! Like [`super::web_fetch`], the obscura interaction runs on a dedicated
//! blocking thread (`current_thread` runtime + `LocalSet`) because obscura
//! futures are `!Send` and our `Tool::execute` future must be `Send`.

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{effective_proxy, json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use std::time::Duration;

use super::research;
use super::web_read::{self, SearchResult};

/// CSS selector that marks a loaded result container, per engine — used to
/// wait for the SERP to render before reading the DOM.
fn wait_selector(engine: &str) -> &'static str {
    match engine {
        "bing" => "li.b_algo",
        "baidu" => "div.c-container",
        "sogou" => "div.vrwrap",
        _ => ".result", // ddg
    }
}

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web (engine: bing | baidu | sogou | ddg, rendered through a \
         headless browser for anti-bot resilience) and return a JSON list of \
         {title, url, snippet} results."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("query".into(), json::prop_str("The search query."));
        props.insert(
            "engine".into(),
            serde_json::json!({
                "type": "string",
                "enum": ["bing", "baidu", "sogou", "ddg"],
                "description": "Search engine to use (default \"ddg\")."
            }),
        );
        props.insert(
            "limit".into(),
            serde_json::json!({ "type": "integer", "description": "Max results to return (1-20, default 8)." }),
        );
        json::object_schema(Value::Object(props), &["query"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(ToolOutput::err("query is required"));
        }
        let engine = input
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("ddg")
            .to_string();
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .clamp(1, 20) as usize;

        let search_url = research::serp_url(&engine, query);
        let search_url_str = search_url.to_string();
        let wait_sel = wait_selector(&engine).to_string();
        let proxy = effective_proxy(ctx.proxy.as_deref());

        let joined = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("worker runtime build failed: {e}"))?;
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                let mut builder = obscura::Browser::builder().stealth(false);
                if let Some(p) = &proxy {
                    builder = builder.proxy(p.clone());
                }
                let browser = builder
                    .build()
                    .map_err(|e| format!("browser build failed: {e}"))?;
                let mut page = browser
                    .new_page()
                    .await
                    .map_err(|e| format!("open page failed: {e}"))?;
                if let Err(e) = page.goto(&search_url_str).await {
                    return Err(format!("search failed: {e}"));
                }
                let _ = page
                    .wait_for_selector(&wait_sel, Duration::from_secs(10))
                    .await;
                let html = page.content();
                drop(page);
                drop(browser);
                Ok(html)
            })
        })
        .await;

        let html = match joined {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return Ok(ToolOutput::err(e)),
            Err(e) => return Ok(ToolOutput::err(format!("worker join failed: {e}"))),
        };

        let results: Vec<SearchResult> = web_read::parse_search_results(&search_url, &html, limit);
        if results.is_empty() {
            return Ok(ToolOutput::err(format!(
                "no results parsed (engine '{engine}' layout may have changed or is blocked)"
            )));
        }
        Ok(truncate_output(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".into()),
            ctx.max_output,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_selector_maps_all_four_engines() {
        assert_eq!(wait_selector("bing"), "li.b_algo");
        assert_eq!(wait_selector("baidu"), "div.c-container");
        assert_eq!(wait_selector("sogou"), "div.vrwrap");
        assert_eq!(wait_selector("ddg"), ".result");
    }

    #[test]
    fn wait_selector_unknown_engine_falls_back_to_ddg() {
        assert_eq!(wait_selector("google"), ".result");
        assert_eq!(wait_selector(""), ".result");
    }

    #[test]
    fn engine_dispatch_uses_research_serp_url() {
        // execute() renders `research::serp_url(engine, query)` and waits on
        // `wait_selector(engine)`; pin that dispatch contract here since the
        // obscura-backed execute path itself needs a real browser.
        assert_eq!(
            research::serp_url("bing", "rust").as_str(),
            "https://cn.bing.com/search?q=rust"
        );
        assert_eq!(
            research::serp_url("baidu", "rust").as_str(),
            "https://www.baidu.com/s?wd=rust"
        );
        assert_eq!(
            research::serp_url("sogou", "rust").as_str(),
            "https://www.sogou.com/web?query=rust"
        );
        assert_eq!(
            research::serp_url("ddg", "rust").as_str(),
            "https://html.duckduckgo.com/html/?q=rust"
        );
        // unknown engine: bing URL (serp_url fallback) + ddg selector
        assert_eq!(
            research::serp_url("google", "rust").as_str(),
            "https://cn.bing.com/search?q=rust"
        );
        assert_eq!(wait_selector("google"), ".result");
    }

    #[test]
    fn parameters_enum_advertises_four_engines() {
        let schema = WebSearchTool.parameters();
        let enum_ = schema["properties"]["engine"]["enum"].as_array().unwrap();
        let engines: Vec<&str> = enum_.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(engines, ["bing", "baidu", "sogou", "ddg"]);
    }
}
