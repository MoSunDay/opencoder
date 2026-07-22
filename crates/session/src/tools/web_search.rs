//! DuckDuckGo HTML search, gated behind the `browser` cargo feature. Loads the
//! DDG HTML results page through obscura (so it survives DDG's anti-bot), then
//! parses `{title, url, snippet}` rows via [`super::web_read::parse_ddg_results`].
//!
//! Like [`super::web_fetch`], the obscura interaction runs on a dedicated
//! blocking thread (`current_thread` runtime + `LocalSet`) because obscura
//! futures are `!Send` and our `Tool::execute` future must be `Send`.

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{effective_proxy, json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use std::time::Duration;
use url::Url;

use super::web_read::{self, SearchResult};

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web via DuckDuckGo (rendered through a headless browser for anti-bot resilience) and return a JSON list of {title, url, snippet} results."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("query".into(), json::prop_str("The search query."));
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
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(8)
            .clamp(1, 20) as usize;

        let search_url = match Url::parse_with_params(DDG_HTML_URL, &[("q", query)]) {
            Ok(u) => u,
            Err(e) => return Ok(ToolOutput::err(format!("bad query: {e}"))),
        };
        let search_url_str = search_url.to_string();
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
                    .wait_for_selector(".result", Duration::from_secs(10))
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

        let results: Vec<SearchResult> = web_read::parse_ddg_results(&html, limit);
        if results.is_empty() {
            return Ok(ToolOutput::err(
                "no results parsed (DDG layout may have changed)",
            ));
        }
        Ok(truncate_output(
            serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".into()),
            ctx.max_output,
        ))
    }
}
