//! Headless Chrome rendering via CLI. Spawns short-lived `chrome --headless`
//! processes for `fetch` (dump DOM + extract readable text) and `screenshot`
//! (capture a full-page PNG). No persistent browser session — each call is
//! independent. Chrome binary is auto-detected from PATH or `$CHROME_PATH`.

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;

use super::web_read;

pub struct ChromeHeadlessTool;

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

async fn do_fetch(input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
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

    let mut cmd = tokio::process::Command::new(&chrome);
    cmd.args([
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--dump-dom",
    ]);
    if let Some(wait) = input
        .get("wait")
        .and_then(|v| v.as_u64())
        .filter(|&w| w > 0)
    {
        cmd.arg(format!("--virtual-time-budget={wait}"));
    }
    cmd.arg(&url);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let output = cmd.output().await;
    match output {
        Ok(o) if o.status.success() => {
            let html = String::from_utf8_lossy(&o.stdout);
            let text = web_read::extract_readable_text(&html);
            let body = format!("# {url}\n\n{text}");
            Ok(truncate_output(body, ctx.max_output))
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            Ok(ToolOutput::err(format!(
                "Chrome exited with {}: {stderr}",
                o.status
            )))
        }
        Err(e) => Ok(ToolOutput::err(format!("Failed to launch Chrome: {e}"))),
    }
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

    let tmp = std::env::temp_dir().join(format!(
        "oc-chrome-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let screenshot_arg = format!("--screenshot={}", tmp.display());

    let mut cmd = tokio::process::Command::new(&chrome);
    cmd.args([
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        &screenshot_arg,
        "--window-size=1920,1080",
    ]);
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
    match output {
        Ok(o) if o.status.success() && tmp.exists() => Ok(ToolOutput::ok(format!(
            "Screenshot saved to: {}\nUse the `read` tool to inspect the image.",
            tmp.display()
        ))),
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
        "Headless Chrome via CLI. Actions: fetch (render URL with JS, extract readable \
         text) and screenshot (capture full-page PNG to a temp file). Requires Chrome \
         or Chromium installed. Prefer web_fetch if available."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "action".into(),
            serde_json::json!({
                "type": "string",
                "enum": ["fetch", "screenshot"],
                "description": "The operation to perform."
            }),
        );
        props.insert("url".into(), json::prop_str("The URL to render."));
        props.insert(
            "wait".into(),
            serde_json::json!({
                "type": "integer",
                "description": "Virtual time budget in ms to wait for JS rendering (fetch/screenshot)."
            }),
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
            "screenshot" => do_screenshot(&input, ctx).await,
            other => Ok(ToolOutput::err(format!(
                "Unknown action '{other}'. Use 'fetch' or 'screenshot'."
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
}
