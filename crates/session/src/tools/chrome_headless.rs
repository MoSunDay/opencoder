//! Headless Chrome rendering via CLI. Spawns short-lived `chrome --headless`
//! processes for `fetch` (dump DOM + extract readable text / structured SERP),
//! `resolve` (unwind search-engine redirect URLs to the real target), `html`
//! (raw rendered DOM for downstream `web_extract`) and `screenshot` (full-page
//! PNG). No persistent browser session — each call is independent. Chrome
//! binary is auto-detected from PATH or `$CHROME_PATH`.

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use url::Url;

use super::{web_extract, web_read};

pub struct ChromeHeadlessTool;

/// Real Chrome desktop UA — search engines and bot walls are far more likely to
/// serve real content to this than to a `HeadlessChrome` UA. Overridable per
/// call via the `ua` input.
const REAL_CHROME_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// Locate a Chrome/Chromium binary. Checks `$CHROME_PATH` first, then common
/// binary names on `$PATH`.
fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CHROME_PATH") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidates = [
        "google-chrome-stable",
        "google-chrome",
        "chromium-browser",
        "chromium",
    ];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in &candidates {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn not_found_msg() -> String {
    "Chrome/Chromium not found. Run ~/.opencoder/install-skills-dep.sh to install, \
     or set the CHROME_PATH environment variable to the binary path."
        .to_string()
}

/// Returns true when `s` (up to the first path/query/fragment delimiter) is
/// a 1-5 digit port number, distinguishing `localhost:3000` from
/// `javascript:alert(1)`.
fn looks_like_port(s: &str) -> bool {
    let port_part = s.split(['/', '?', '#']).next().unwrap_or("");
    !port_part.is_empty() && port_part.len() <= 5 && port_part.chars().all(|c| c.is_ascii_digit())
}

/// Normalise a user-supplied URL: add `https://` when no scheme is present.
/// Rejects non-http(s) schemes (e.g. `file://`, `ftp://`, `javascript:`,
/// `data:`) to prevent local file reads and other scheme-based attacks.
fn normalise_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if let Some(colon) = trimmed.find(':') {
        let before = &trimmed[..colon];
        let after = &trimmed[colon + 1..];
        // A URL scheme is an alphabetic token before ':'. However, a
        // hostname:port pair (e.g. `localhost:3000`) also matches this shape,
        // so we check whether what follows ':' looks like a port number
        // (1-5 digits). If it does, treat it as host:port, not a scheme.
        if !before.is_empty()
            && before.chars().all(|c| c.is_ascii_alphabetic())
            && !looks_like_port(after)
        {
            let scheme = before.to_lowercase();
            if scheme != "http" && scheme != "https" {
                return Err(format!(
                    "Unsupported URL scheme '{scheme}'. Only http and https are \
                     allowed (file://, ftp://, javascript:, etc. are blocked \
                     for security)."
                ));
            }
            return Ok(trimmed.to_string());
        }
    }
    Ok(format!("https://{trimmed}"))
}

/// Shared Chrome flags: container-safe sandbox/gpu flags plus anti-detection
/// (real UA, `AutomationControlled` off) and a zh-CN locale so Chinese sites
/// and search engines behave like a normal browser.
fn chrome_base_args(user_agent: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--headless=new".to_string(),
        "--no-sandbox".to_string(),
        "--disable-gpu".to_string(),
        "--disable-blink-features=AutomationControlled".to_string(),
        "--lang=zh-CN".to_string(),
    ];
    let ua = user_agent.unwrap_or(REAL_CHROME_UA);
    if !ua.is_empty() {
        args.push(format!("--user-agent={ua}"));
    }
    args
}

/// A fresh per-call profile dir. Required by snap-packaged Chromium in
/// containers and avoids "profile in use" races when calls overlap.
fn temp_profile_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "oc-chrome-profile-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ))
}

/// Render `url` with headless Chrome and return the final DOM. Shared by
/// `fetch`, `resolve` and `html`; `wait_ms` maps to `--virtual-time-budget`
/// and `ua` overrides the default Chrome UA.
pub(crate) async fn dump_dom(
    url: &str,
    wait_ms: Option<u64>,
    ua: Option<&str>,
) -> Result<String, String> {
    let chrome = find_chrome().ok_or_else(not_found_msg)?;
    let profile = temp_profile_dir();
    let _ = std::fs::create_dir_all(&profile);

    let mut cmd = tokio::process::Command::new(&chrome);
    cmd.args(chrome_base_args(ua));
    cmd.arg("--dump-dom");
    cmd.arg(format!("--user-data-dir={}", profile.display()));
    if let Some(wait) = wait_ms.filter(|&w| w > 0) {
        cmd.arg(format!("--virtual-time-budget={wait}"));
    }
    cmd.arg(url);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await;
    let _ = std::fs::remove_dir_all(&profile);
    match output {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(format!(
            "Chrome exited with {}: {}",
            o.status,
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("Failed to launch Chrome: {e}")),
    }
}

/// Recognise search-engine redirect URLs (Baidu/Sogou `/link?url=...`) whose
/// real target is not decodable client-side and must be rendered to unwind.
fn is_redirect_url(raw: &str) -> bool {
    let Ok(u) = Url::parse(raw) else {
        return false;
    };
    let host = u.host_str().unwrap_or("");
    (host.contains("baidu.com") || host.contains("sogou.com"))
        && u.path().contains("/link")
        && u.query().map(|q| q.contains("url=")).unwrap_or(false)
}

/// Extract the real target URL from a rendered redirect page: prefer
/// `<link rel="canonical">`, then `<meta property="og:url">`, else the URL
/// that was actually rendered. Relative canonical hrefs are resolved against
/// `fallback`.
pub(crate) fn extract_final_url(html: &str, fallback: &str) -> String {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    for sel_str in ["link[rel='canonical']", "meta[property='og:url']"] {
        let Ok(sel) = Selector::parse(sel_str) else {
            continue;
        };
        let Some(el) = doc.select(&sel).next() else {
            continue;
        };
        let href = el
            .attr("href")
            .or_else(|| el.attr("content"))
            .unwrap_or("")
            .trim();
        if href.is_empty() {
            continue;
        }
        if let Ok(u) = base_join(fallback, href) {
            return u.to_string();
        }
        return href.to_string();
    }
    fallback.to_string()
}

fn base_join(base: &str, href: &str) -> Result<Url, url::ParseError> {
    Url::parse(base).and_then(|b| b.join(href))
}

/// Render structured SERP results as a clean numbered markdown list. The header
/// echoes the source URL so callers can tell which query produced the rows; the
/// snippet and url lines are omitted when empty.
fn format_serp_output(url: &str, results: &[web_read::SearchResult]) -> String {
    let mut out = format!("# Search results: {url}\n");
    for (i, r) in results.iter().enumerate() {
        let n = i + 1;
        out.push_str(&format!("\n{n}. **{}**\n", r.title));
        if !r.snippet.is_empty() {
            out.push_str(&format!("   {}\n", r.snippet));
        }
        if !r.url.is_empty() {
            out.push_str(&format!("   {}\n", r.url));
        }
    }
    out
}

async fn do_fetch(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let raw_url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if raw_url.is_empty() {
        return Ok(ToolOutput::err("Missing required parameter: url."));
    }
    let url = match normalise_url(raw_url) {
        Ok(u) => u,
        Err(msg) => return Ok(ToolOutput::err(msg)),
    };
    let wait = input.get("wait").and_then(|v| v.as_u64());
    let ua = input
        .get("ua")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let html = match dump_dom(&url, wait, ua).await {
        Ok(h) => h,
        Err(e) => return Ok(ToolOutput::err(e)),
    };
    // Auto-detect SERP pages and emit structured results; otherwise fall back
    // to readable-text extraction. Keeps non-search pages unchanged.
    let parsed_url = Url::parse(&url).ok();
    let serp = parsed_url
        .as_ref()
        .map(|u| web_read::parse_search_results(u, &html, 12))
        .filter(|v| !v.is_empty());
    let body = match serp {
        Some(results) => format_serp_output(&url, &results),
        None => {
            let text = web_read::extract_readable_text(&html);
            format!("# {url}\n\n{text}")
        }
    };
    Ok(truncate_output(body, ctx.max_output))
}

/// Render a (possibly search-engine redirect) URL and report the real target
/// from the rendered page's canonical/og:url plus a title and short excerpt —
/// the key step that makes Baidu/Sogou SERP links actually usable.
async fn do_resolve(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let raw_url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if raw_url.is_empty() {
        return Ok(ToolOutput::err("Missing required parameter: url."));
    }
    let url = match normalise_url(raw_url) {
        Ok(u) => u,
        Err(msg) => return Ok(ToolOutput::err(msg)),
    };
    let ua = input
        .get("ua")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    // Redirect links need a moment to bounce to the real target.
    let wait = input
        .get("wait")
        .and_then(|v| v.as_u64())
        .or_else(|| is_redirect_url(&url).then_some(2000));
    let html = match dump_dom(&url, wait, ua).await {
        Ok(h) => h,
        Err(e) => return Ok(ToolOutput::err(e)),
    };
    let final_url = extract_final_url(&html, &url);
    let final_parsed = Url::parse(&final_url).unwrap_or_else(|_| Url::parse(&url).unwrap());
    let article = web_extract::extract_article(&html, &final_parsed);
    let mut body = format!("# Resolved URL: {final_url}\n");
    if !article.title.is_empty() {
        body.push_str(&format!("**{}**\n", article.title));
    }
    let excerpt: String = article.content.chars().take(600).collect();
    if !excerpt.is_empty() {
        body.push_str(&excerpt);
        if article.content.chars().count() > 600 {
            body.push_str("\n…");
        }
    }
    Ok(truncate_output(body, ctx.max_output))
}

/// Return the raw rendered DOM (truncated) so callers can run `web_extract`
/// or any other parser over it.
async fn do_html(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    let raw_url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if raw_url.is_empty() {
        return Ok(ToolOutput::err("Missing required parameter: url."));
    }
    let url = match normalise_url(raw_url) {
        Ok(u) => u,
        Err(msg) => return Ok(ToolOutput::err(msg)),
    };
    let wait = input.get("wait").and_then(|v| v.as_u64());
    let ua = input
        .get("ua")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let html = match dump_dom(&url, wait, ua).await {
        Ok(h) => h,
        Err(e) => return Ok(ToolOutput::err(e)),
    };
    Ok(truncate_output(
        format!("<!-- raw DOM of {url} -->\n{html}"),
        ctx.max_output,
    ))
}

async fn do_screenshot(input: &Value, _ctx: &ToolContext) -> Result<ToolOutput> {
    let raw_url = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if raw_url.is_empty() {
        return Ok(ToolOutput::err("Missing required parameter: url."));
    }
    let url = match normalise_url(raw_url) {
        Ok(u) => u,
        Err(msg) => return Ok(ToolOutput::err(msg)),
    };
    let chrome = match find_chrome() {
        Some(c) => c,
        None => return Ok(ToolOutput::err(not_found_msg())),
    };
    let ua = input
        .get("ua")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let tmp = std::env::temp_dir().join(format!(
        "oc-chrome-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let profile = temp_profile_dir();
    let _ = std::fs::create_dir_all(&profile);
    let screenshot_arg = format!("--screenshot={}", tmp.display());

    let mut cmd = tokio::process::Command::new(&chrome);
    cmd.args(chrome_base_args(ua));
    cmd.args([&screenshot_arg, "--window-size=1920,1080"]);
    cmd.arg(format!("--user-data-dir={}", profile.display()));
    if let Some(wait) = input
        .get("wait")
        .and_then(|v| v.as_u64())
        .filter(|&w| w > 0)
    {
        cmd.arg(format!("--virtual-time-budget={wait}"));
    }
    cmd.arg(&url);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await;
    let _ = std::fs::remove_dir_all(&profile);
    match output {
        Ok(o) if o.status.success() && tmp.exists() => {
            let images = match super::image_data::file_to_data_uri(&tmp) {
                Ok(uri) => vec![uri],
                Err(_) => Vec::new(),
            };
            Ok(ToolOutput::ok_with_images(
                format!(
                    "Screenshot of {} captured and saved to: {}",
                    url,
                    tmp.display()
                ),
                images,
            ))
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            Ok(ToolOutput::err(format!(
                "Chrome screenshot failed (exit {}): {stderr}",
                o.status
            )))
        }
        Err(e) => Ok(ToolOutput::err(format!("Failed to launch Chrome: {e}"))),
    }
}

#[async_trait]
impl Tool for ChromeHeadlessTool {
    fn name(&self) -> &str {
        "chrome_headless"
    }
    fn description(&self) -> &str {
        "Headless Chrome via CLI. Actions: fetch (render URL with JS, extract \
         readable text / structured SERP), resolve (unwind Baidu/Sogou redirect \
         links to the real URL + title + excerpt), html (dump raw rendered DOM \
         for web_extract), screenshot (full-page PNG). Requires Chrome or \
         Chromium installed."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "action".into(),
            serde_json::json!({
                "type": "string",
                "enum": ["fetch", "resolve", "html", "screenshot"],
                "description": "The operation to perform."
            }),
        );
        props.insert("url".into(), json::prop_str("The URL to render."));
        props.insert(
            "wait".into(),
            serde_json::json!({
                "type": "integer",
                "description": "Virtual time budget in ms to wait for JS rendering (fetch/resolve/html/screenshot)."
            }),
        );
        props.insert(
            "ua".into(),
            json::prop_str("Optional override for the default Chrome user-agent string."),
        );
        json::object_schema(Value::Object(props), &["action", "url"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("fetch");
        match action {
            "fetch" => do_fetch(&input, ctx).await,
            "resolve" => do_resolve(&input, ctx).await,
            "html" => do_html(&input, ctx).await,
            "screenshot" => do_screenshot(&input, ctx).await,
            other => Ok(ToolOutput::err(format!(
                "Unknown action '{other}'. Use 'fetch', 'resolve', 'html' or 'screenshot'."
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_url_adds_scheme() {
        assert_eq!(normalise_url("example.com").unwrap(), "https://example.com");
        assert_eq!(normalise_url("http://local").unwrap(), "http://local");
        assert_eq!(normalise_url("  https://x.com  ").unwrap(), "https://x.com");
    }

    #[test]
    fn normalise_url_rejects_file_scheme() {
        assert!(normalise_url("file:///etc/passwd").is_err());
        assert!(normalise_url("file://localhost/etc/passwd").is_err());
    }

    #[test]
    fn normalise_url_rejects_other_dangerous_schemes() {
        assert!(normalise_url("ftp://evil.com/file").is_err());
        assert!(normalise_url("javascript:alert(1)").is_err());
        assert!(normalise_url("data:text/html,<script>").is_err());
    }

    #[test]
    fn normalise_url_accepts_http_and_https() {
        assert!(normalise_url("http://example.com").is_ok());
        assert!(normalise_url("HTTPS://example.com").is_ok());
    }

    #[test]
    fn normalise_url_host_port_not_rejected() {
        // Hostname:port pairs should NOT be treated as URL schemes.
        assert!(normalise_url("localhost:3000").is_ok());
        assert!(normalise_url("example.com:8080").is_ok());
    }

    #[test]
    fn not_found_message_is_helpful() {
        let msg = not_found_msg();
        assert!(msg.contains("install-skills-dep.sh"));
        assert!(msg.contains("CHROME_PATH"));
    }

    #[test]
    fn format_serp_output_renders_markdown_list() {
        let results = vec![
            web_read::SearchResult {
                title: "First".to_string(),
                url: "http://www.baidu.com/link?url=a".to_string(),
                snippet: "snippet one".to_string(),
            },
            web_read::SearchResult {
                title: "Second".to_string(),
                url: String::new(),
                snippet: String::new(),
            },
        ];
        let out = format_serp_output("https://www.baidu.com/s?wd=x", &results);
        assert!(out.starts_with("# Search results: https://www.baidu.com/s?wd=x\n"));
        // numbered starting at 1
        assert!(out.contains("\n1. **First**\n"));
        assert!(out.contains("\n2. **Second**\n"));
        // snippet + url printed for first row
        assert!(out.contains("   snippet one\n"));
        assert!(out.contains("   http://www.baidu.com/link?url=a\n"));
        // empty fields are omitted (no url line for second row)
        assert!(!out.contains("**Second**\n   \n"));
    }

    #[test]
    fn base_args_use_real_ua_and_anti_detection() {
        let args = chrome_base_args(None);
        let joined = args.join(" ");
        assert!(joined.contains("--headless=new"));
        assert!(joined.contains("--no-sandbox"));
        assert!(joined.contains("--disable-blink-features=AutomationControlled"));
        assert!(joined.contains("--lang=zh-CN"));
        assert!(joined.contains("--user-agent=Mozilla/5.0"));
        assert!(!joined.contains("HeadlessChrome"));
        // ua override replaces the default
        let custom = chrome_base_args(Some("CustomUA/1.0"));
        assert!(custom.iter().any(|a| a == "--user-agent=CustomUA/1.0"));
        assert!(!custom.iter().any(|a| a.contains("Mozilla/5.0")));
    }

    #[test]
    fn redirect_urls_are_recognised() {
        assert!(is_redirect_url(
            "https://www.baidu.com/link?url=https%3A%2F%2Freal.example%2Fx&wd=&eqid=1"
        ));
        assert!(is_redirect_url("https://www.sogou.com/link?url=abc"));
        assert!(!is_redirect_url("https://www.baidu.com/s?wd=rust"));
        assert!(!is_redirect_url("https://example.com/article/1"));
        assert!(!is_redirect_url("not a url"));
    }

    #[test]
    fn extract_final_url_prefers_canonical() {
        let html = "<html><head><link rel=\"canonical\" href=\"https://real.example/post\">\
                    <meta property=\"og:url\" content=\"https://og.example/post\"></head></html>";
        assert_eq!(
            extract_final_url(html, "https://www.baidu.com/link?url=x"),
            "https://real.example/post"
        );
    }

    #[test]
    fn extract_final_url_falls_back_to_og_url() {
        let html = "<html><head><meta property=\"og:url\" content=\"https://og.example/post\"></head></html>";
        assert_eq!(
            extract_final_url(html, "https://www.baidu.com/link?url=x"),
            "https://og.example/post"
        );
    }

    #[test]
    fn extract_final_url_resolves_relative_canonical() {
        let html = "<html><head><link rel=\"canonical\" href=\"/posts/42\"></head></html>";
        assert_eq!(
            extract_final_url(html, "https://real.example/a/b"),
            "https://real.example/posts/42"
        );
    }

    #[test]
    fn extract_final_url_returns_fallback_when_absent() {
        let html = "<html><head><title>no canonical</title></head><body>hi</body></html>";
        assert_eq!(
            extract_final_url(html, "https://example.com/x"),
            "https://example.com/x"
        );
    }

    #[test]
    fn parameters_schema_advertises_all_actions() {
        // resolve/html (and screenshot) dispatch live in execute(); the
        // parameter schema is the pure, unit-testable half of that contract.
        let schema = ChromeHeadlessTool.parameters();
        let enum_ = schema["properties"]["action"]["enum"].as_array().unwrap();
        let actions: Vec<&str> = enum_.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(actions, ["fetch", "resolve", "html", "screenshot"]);
        let required = schema["required"].as_array().unwrap();
        let required: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(required, ["action", "url"]);
    }
}
