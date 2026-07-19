//! Content-extraction algorithm ported from agent-browser's `cli/src/read.rs`
//! (github.com/vercel-labs/agent-browser). The original is a hand-written
//! HTML char-parser; we keep the *algorithm* (markdown Accept negotiation,
//! `.md` path fallback, `llms.txt` / `llms-full.txt` ancestor crawl, 2 MB body
//! cap, readable-text extraction) but delegate HTML->text to the `html2text`
//! crate and URL parsing/selection to `scraper` for maintainability.
//!
//! These functions are pure and feature-independent, so they compile and are
//! unit-tested in the default (no-`browser`) build. The feature-gated
//! `web_fetch` tool renders a URL with obscura and then feeds the resulting
//! HTML through [`extract_readable_text`].

use url::Url;

/// Hard cap on fetched body size, mirroring agent-browser's `BODY_LIMIT`.
pub const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Accept header value expressing a markdown preference, mirroring agent-browser
/// (`text/markdown` first, then plain, then HTML, then anything).
pub const READ_ACCEPT: &str = "text/markdown, text/plain;q=0.9, text/html;q=0.7, */*;q=0.1";

/// Normalize a user-supplied URL: add an `https://` scheme when missing, reject
/// non-http(s) schemes, drop the fragment. Mirrors `normalize_url` in read.rs.
pub fn normalize_url(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };
    let mut url = Url::parse(&candidate).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported URL scheme '{}': use http or https",
            url.scheme()
        ));
    }
    let host = url.host_str().unwrap_or("");
    if host.is_empty() {
        return Err("URL must include a host".to_string());
    }
    url.set_fragment(None);
    Ok(url)
}

/// Build the `.md` sibling URL used as a markdown fallback. Mirrors
/// `markdown_fallback_url` in read.rs: append `.md` to the path (or use
/// `/index.md` at the root); returns `None` if the path already ends in `.md`.
/// Query/fragment are preserved.
pub fn markdown_fallback_url(url: &Url) -> Option<Url> {
    if url.path().ends_with(".md") {
        return None;
    }
    let mut md = url.clone();
    let path = url.path();
    let next_path = if path == "/" || path.is_empty() {
        "/index.md".to_string()
    } else {
        format!("{}.md", path.trim_end_matches('/'))
    };
    md.set_path(&next_path);
    Some(md)
}

/// Ancestor crawl for an `llms.txt`-style file. For a target like
/// `https://site.com/a/b/c?x=1`, `llms.txt` candidates are searched
/// deepest-first up to the root: `/a/b/llms.txt`, `/a/llms.txt`, `/llms.txt`.
/// Mirrors `llms_file_candidates` in read.rs.
pub fn llms_txt_candidates(url: &Url, filename: &str) -> Vec<Url> {
    let mut base = url.clone();
    base.set_query(None);
    base.set_fragment(None);
    let segments: Vec<&str> = base
        .path()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let n = segments.len();
    let mut out = Vec::new();
    // Treat the last path segment as a file and walk its directory up to the
    // root, emitting `<dir>/<filename>` deepest-first. `take` = number of
    // leading segments kept as the directory. `n.max(1)` ensures a root URL
    // (no path segments) still yields the single `/llms.txt` root candidate.
    for take in (0..n.max(1)).rev() {
        let dir = segments.get(..take).map(|s| s.join("/")).unwrap_or_default();
        let p = if dir.is_empty() {
            format!("/{filename}")
        } else {
            format!("/{dir}/{filename}")
        };
        let mut cand = base.clone();
        cand.set_path(&p);
        out.push(cand);
    }
    out
}

/// Extract readable text from an HTML document, mirroring agent-browser's
/// `html_to_markdownish`: prefer `<main>` / `<article>` / `[role=main]`,
/// otherwise `<body>`, then convert to text with `html2text` (which drops
/// `<script>`/`<style>`). Trailing blank lines are collapsed.
pub fn extract_readable_text(html: &str) -> String {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let main = Selector::parse("main, article, [role='main']").unwrap();
    let body = Selector::parse("body").unwrap();
    let source: String = if let Some(el) = doc.select(&main).next() {
        el.inner_html()
    } else if let Some(el) = doc.select(&body).next() {
        el.inner_html()
    } else {
        html.to_string()
    };
    let raw = html2text::from_read(source.as_bytes(), 100).unwrap_or_default();
    collapse_blank_lines(raw.trim())
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            blanks += 1;
            if blanks <= 1 {
                out.push('\n');
            }
        } else {
            blanks = 0;
            out.push_str(t);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// A single search result row parsed from a DuckDuckGo HTML results page.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Parse DuckDuckGo's `html.duckduckgo.com/html/` results page into
/// `{title, url, snippet}` rows. DDG wraps result links in a redirect
/// (`//duckduckgo.com/l/?uddg=<encoded>`); we unwrap the real target from the
/// `uddg` query param. Pure + obscura-free so it is unit-tested in the default
/// build with a frozen fixture.
pub fn parse_ddg_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let result_sel = Selector::parse(".result").unwrap_or_else(|e| panic!("bad sel: {e:?}"));
    let link_sel = Selector::parse(".result__a").unwrap();
    let snip_sel = Selector::parse(".result__snippet").unwrap();
    let mut out = Vec::new();
    for r in doc.select(&result_sel) {
        let Some(a) = r.select(&link_sel).next() else { continue };
        let title: String = a
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let href = a.value().attr("href").unwrap_or("");
        let url = decode_ddg_href(href);
        let snippet = r
            .select(&snip_sel)
            .next()
            .map(|s| {
                s.text()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .trim()
                    .to_string()
            })
            .unwrap_or_default();
        out.push(SearchResult { title, url, snippet });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Unwrap a DDG redirect link to the real target URL. DDG result anchors look
/// like `//duckduckgo.com/l/?uddg=https%3A%2F%2Freal&rut=...`; extract the
/// `uddg` param. Non-redirect hrefs are returned (protocol-fixed) as-is.
fn decode_ddg_href(href: &str) -> String {
    let full = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    if let Ok(u) = Url::parse(&full) {
        if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "uddg") {
            return v.to_string();
        }
        return u.to_string();
    }
    full
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_https_and_strips_fragment() {
        let u = normalize_url("example.com/a/b#frag").unwrap();
        assert_eq!(u.as_str(), "https://example.com/a/b");
        assert!(normalize_url("example.com").is_ok());
    }

    #[test]
    fn normalize_rejects_non_http() {
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("ftp://h/x").is_err());
    }

    #[test]
    fn normalize_requires_host() {
        // The `url` crate rejects truly host-less special-scheme URLs at parse
        // time; normalize_url must surface that as an error.
        assert!(normalize_url("https://").is_err());
        assert!(normalize_url("https:///").is_err());
        // Sanity: a bare host that url crate recovers from stays valid.
        assert!(normalize_url("https://no-host").is_ok());
    }

    #[test]
    fn markdown_fallback_appends_md() {
        let u = normalize_url("https://site.com/docs/intro").unwrap();
        let md = markdown_fallback_url(&u).unwrap();
        assert_eq!(md.path(), "/docs/intro.md");
        // query preserved, fragment dropped
        let uq = normalize_url("https://site.com/a?x=1").unwrap();
        let mdq = markdown_fallback_url(&uq).unwrap();
        assert_eq!(mdq.path(), "/a.md");
        assert_eq!(mdq.query(), Some("x=1"));
    }

    #[test]
    fn markdown_fallback_root_uses_index() {
        let u = normalize_url("https://site.com/").unwrap();
        let md = markdown_fallback_url(&u).unwrap();
        assert_eq!(md.path(), "/index.md");
    }

    #[test]
    fn markdown_fallback_none_when_already_md() {
        let u = normalize_url("https://site.com/a/b.md").unwrap();
        assert!(markdown_fallback_url(&u).is_none());
    }

    #[test]
    fn llms_candidates_crawl_to_root() {
        let u = normalize_url("https://site.com/a/b/c").unwrap();
        let cands: Vec<String> = llms_txt_candidates(&u, "llms.txt")
            .iter()
            .map(|u| u.path().to_string())
            .collect();
        assert_eq!(cands, vec!["/a/b/llms.txt", "/a/llms.txt", "/llms.txt"]);
    }

    #[test]
    fn llms_candidates_root_target() {
        let u = normalize_url("https://site.com/").unwrap();
        let cands: Vec<String> = llms_txt_candidates(&u, "llms.txt")
            .iter()
            .map(|u| u.path().to_string())
            .collect();
        assert_eq!(cands, vec!["/llms.txt"]);
    }

    #[test]
    fn extract_readable_prefers_main_over_body() {
        let html = r#"<html><body>NAV<nav>x</nav><main><h1>Title</h1><p>Hello world</p></main></body></html>"#;
        let text = extract_readable_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
        // script/style content must not leak.
        let with_script = r#"<html><body><script>var x=1;</script><p>visible</p></body></html>"#;
        assert!(!extract_readable_text(with_script).contains("var x=1"));
        assert!(extract_readable_text(with_script).contains("visible"));
    }

    #[test]
    fn extract_readable_drops_script_and_style() {
        let html = r#"<html><body><style>.a{color:red}</style><p>keep me</p></body></html>"#;
        let t = extract_readable_text(html);
        assert!(t.contains("keep me"));
        assert!(!t.contains("color:red"));
    }

    #[test]
    fn extract_readable_collapses_blank_lines() {
        let html = r#"<html><body><p>a</p><p>b</p><p>c</p></body></html>"#;
        let t = extract_readable_text(html);
        // no run of more than one blank line.
        assert!(!t.contains("\n\n\n"));
    }

    const DDG_FIXTURE: &str = r#"<html><body>
<div class="results">
  <div class="result">
    <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Ffoo&rut=abc">Example Foo</a></h2>
    <a class="result__snippet">The foo snippet text.</a>
  </div>
  <div class="result">
    <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fbar.io&rut=x">Bar</a></h2>
    <a class="result__snippet">Bar snippet.</a>
  </div>
  <div class="result projects">
    <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fskip.me">NoTitleSkip</a></h2>
  </div>
</div></body></html>"#;

    #[test]
    fn parse_ddg_extracts_title_url_snippet() {
        let r = parse_ddg_results(DDG_FIXTURE, 8);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].title, "Example Foo");
        assert_eq!(r[0].url, "https://example.com/foo");
        assert_eq!(r[0].snippet, "The foo snippet text.");
        assert_eq!(r[1].url, "https://bar.io");
    }

    #[test]
    fn parse_ddg_respects_limit() {
        let r = parse_ddg_results(DDG_FIXTURE, 1);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "Example Foo");
    }

    #[test]
    fn parse_ddg_handles_empty_and_non_ddg_href() {
        let empty = parse_ddg_results("<html></html>", 5);
        assert!(empty.is_empty());
        // a plain (non-redirect) href passes through protocol-fixed.
        let html = r#"<div class="result"><a class="result__a" href="//site.org/x">S</a><a class="result__snippet">s</a></div>"#;
        let r = parse_ddg_results(html, 5);
        assert_eq!(r[0].url, "https://site.org/x");
    }
}
