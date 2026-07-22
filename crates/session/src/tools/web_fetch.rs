//! Browser-driven web fetch, gated behind the `browser` cargo feature. Renders
//! `url` with the obscura headless browser (real JS execution + anti-crawl +
//! proxy support), then extracts readable text via [`super::web_read`]. Use for
//! pages that 403 plain HTTP clients or need JS to render their content.
//!
//! obscura (deno_core / V8) is fundamentally single-threaded: its futures are
//! `!Send`. Our `Tool::execute` contract requires a `Send` future, so the whole
//! obscura interaction runs on a dedicated blocking thread under a
//! `current_thread` runtime + `LocalSet`. Only `Send` data (`String`, options)
//! crosses the `spawn_blocking` boundary, keeping the outer future `Send`.

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{effective_proxy, json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use std::time::Duration;

use super::web_read::{self, BODY_LIMIT};

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Fetch a URL via a real headless browser (JS rendering, proxy-aware) and return readable text extracted from the rendered HTML. Prefers <main>/<article>. Use for pages that block plain HTTP clients (403/anti-bot) or need JavaScript."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("url".into(), json::prop_str("The http(s) URL to fetch."));
        props.insert(
            "wait_selector".into(),
            serde_json::json!({ "type": "string", "description": "Optional CSS selector to wait for before extracting content (for JS-heavy pages)." }),
        );
        json::object_schema(Value::Object(props), &["url"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let raw = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let url = match web_read::normalize_url(raw) {
            Ok(u) => u,
            Err(e) => return Ok(ToolOutput::err(e)),
        };
        let proxy = effective_proxy(ctx.proxy.as_deref());
        let wait_selector = input
            .get("wait_selector")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let url_str = url.to_string();

        // Run the !Send obscura work on a dedicated blocking thread with its
        // own single-threaded runtime + LocalSet. The closure owns only `Send`
        // data; the returned `(html, final_url)` is `Send`.
        let joined = tokio::task::spawn_blocking(
            move || -> std::result::Result<(String, String), String> {
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
                    if let Err(e) = page.goto(&url_str).await {
                        return Err(format!("navigate failed: {e}"));
                    }
                    if let Some(sel) = &wait_selector {
                        let _ = page.wait_for_selector(sel, Duration::from_secs(15)).await;
                    }
                    let html = page.content();
                    let final_url = page.url();
                    drop(page);
                    drop(browser);
                    Ok((html, final_url))
                })
            },
        )
        .await;

        let (html, final_url) = match joined {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => return Ok(ToolOutput::err(e)),
            Err(e) => return Ok(ToolOutput::err(format!("worker join failed: {e}"))),
        };

        let mut text = web_read::extract_readable_text(&html);
        if text.len() > BODY_LIMIT {
            text.truncate(BODY_LIMIT);
            text.push_str("\n\n[truncated at 2 MB]");
        }
        let body = format!("# {final_url}\n\n{text}");
        Ok(truncate_output(body, ctx.max_output))
    }
}
