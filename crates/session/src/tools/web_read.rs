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
    let segments: Vec<&str> = base.path().split('/').filter(|s| !s.is_empty()).collect();
    let n = segments.len();
    let mut out = Vec::new();
    // Treat the last path segment as a file and walk its directory up to the
    // root, emitting `<dir>/<filename>` deepest-first. `take` = number of
    // leading segments kept as the directory. `n.max(1)` ensures a root URL
    // (no path segments) still yields the single `/llms.txt` root candidate.
    for take in (0..n.max(1)).rev() {
        let dir = segments
            .get(..take)
            .map(|s| s.join("/"))
            .unwrap_or_default();
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
    let result_sel = Selector::parse(".result").unwrap();
    let link_sel = Selector::parse(".result__a").unwrap();
    let snip_sel = Selector::parse(".result__snippet").unwrap();
    let mut out = Vec::new();
    for r in doc.select(&result_sel) {
        let Some(a) = r.select(&link_sel).next() else {
            continue;
        };
        let title: String = a.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if title.is_empty() {
            continue;
        }
        let href = a.value().attr("href").unwrap_or("");
        let url = decode_ddg_href(href);
        let snippet = r
            .select(&snip_sel)
            .next()
            .map(|s| s.text().collect::<Vec<_>>().join(" ").trim().to_string())
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            snippet,
        });
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

/// Normalize whitespace in extracted text: replace non-breaking spaces
/// (`\u{a0}`, what `&nbsp;` decodes to under scraper's `text()`) with a normal
/// space, then collapse all runs of whitespace into single spaces (and trim).
/// `&nbsp;` is pervasive in Baidu SERPs and must not leak as a stray `\u{a0}`.
fn normalize_ws(s: &str) -> String {
    s.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Unescape `&amp;` → `&` in a Baidu redirect href and trim surrounding space.
/// The real target is not decodable client-side, so we keep the redirect href
/// verbatim apart from this one entity fix. Returns empty for a missing href.
fn normalize_baidu_href(href: &str) -> String {
    href.trim().replace("&amp;", "&")
}

/// Make a possibly-relative href absolute against `base` (e.g.
/// `https://www.sogou.com`), then unescape `&amp;` → `&`. Empty stays empty.
/// Used by engines (Bing, Sogou) whose result anchors may be relative redirect
/// links; for engines with already-absolute hrefs pass an empty `base`.
fn normalize_redirect_href(href: &str, base: &str) -> String {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let full = if trimmed.starts_with('/') {
        format!("{base}{trimmed}")
    } else {
        trimmed.to_string()
    };
    full.replace("&amp;", "&")
}

/// First selector in `sels` that matches inside `scope`, with its text
/// normalized; returns `None` when none match. Lets an engine try several
/// candidate snippet containers in priority order.
fn first_snippet(scope: scraper::ElementRef, sels: &[scraper::Selector]) -> Option<String> {
    sels.iter()
        .find_map(|sel| scope.select(sel).next())
        .map(|el| normalize_ws(&el.text().collect::<Vec<_>>().join("")))
}

/// Fallback snippet: the container's full text with the leading title block
/// (located via `title_sel`, e.g. `h2`/`h3`) stripped off the front. Mirrors
/// baidu's own fallback so Bing/Sogou behave consistently.
fn container_text_minus_title(
    scope: &scraper::ElementRef,
    title_sel: &scraper::Selector,
) -> String {
    let full = normalize_ws(&scope.text().collect::<Vec<_>>().join(""));
    let title_text = scope
        .select(title_sel)
        .next()
        .map(|h| normalize_ws(&h.text().collect::<Vec<_>>().join("")))
        .unwrap_or_default();
    if !title_text.is_empty() && full.starts_with(&title_text) {
        full[title_text.len()..].trim_start().to_string()
    } else {
        full
    }
}

/// Parse a Baidu results page into `{title, url, snippet}` rows. Every result
/// (organic `result` and one-box `result-op`) is wrapped in a `div.c-container`;
/// the title lives in the first `h3 a`, snippets in `.c-abstract` (or, failing
/// that, the container's own text with the title block stripped off the front).
/// Result links are Baidu redirects (`http://www.baidu.com/link?url=...` or
/// `baidu.php?url=...`) whose real target is not decodable client-side, so we
/// keep the redirect href as-is. Pure + obscura-free so it is unit-tested in the
/// default build with an inline fixture.
pub fn parse_baidu_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let container_sel = Selector::parse("div.c-container").unwrap();
    let h3a_sel = Selector::parse("h3 a").unwrap();
    let h3_sel = Selector::parse("h3").unwrap();
    let abstract_sel = Selector::parse(".c-abstract").unwrap();
    let mut out = Vec::new();
    let mut seen_titles = std::collections::HashSet::new();
    for container in doc.select(&container_sel) {
        let Some(a) = container.select(&h3a_sel).next() else {
            continue;
        };
        let title = normalize_ws(&a.text().collect::<Vec<_>>().join(""));
        if title.is_empty() {
            continue;
        }
        // dedup by title to drop repeated ad rows.
        if !seen_titles.insert(title.clone()) {
            continue;
        }
        let href = a.value().attr("href").unwrap_or("");
        let url = normalize_baidu_href(href);
        if url.is_empty() {
            continue;
        }
        let snippet = if let Some(ab) = container.select(&abstract_sel).next() {
            normalize_ws(&ab.text().collect::<Vec<_>>().join(""))
        } else {
            // take the container's full text and strip the leading h3 title text.
            container_text_minus_title(&container, &h3_sel)
        };
        out.push(SearchResult {
            title,
            url,
            snippet,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Parse a Bing results page (cn.bing.com / bing.com) into `{title, url,
/// snippet}` rows. Organic results live in `li.b_algo`; titles sit under
/// `h2 a` whose `href` is the *direct* target URL (no redirect unwrap needed),
/// and snippets under `p.b_lineclamp*` (falling back to `.b_caption p`, then
/// the container's own text with the `h2` title stripped off the front). Pure +
/// obscura-free so it is unit-tested in the default build with a frozen fixture.
pub fn parse_bing_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let container_sel = Selector::parse("li.b_algo").unwrap();
    let h2a_sel = Selector::parse("h2 a").unwrap();
    let h2_sel = Selector::parse("h2").unwrap();
    let snippet_sels = [
        Selector::parse("p.b_lineclamp1, p.b_lineclamp2, p.b_lineclamp3").unwrap(),
        Selector::parse(".b_caption p").unwrap(),
    ];
    let mut out = Vec::new();
    let mut seen_titles = std::collections::HashSet::new();
    for container in doc.select(&container_sel) {
        let Some(a) = container.select(&h2a_sel).next() else {
            continue;
        };
        let title = normalize_ws(&a.text().collect::<Vec<_>>().join(""));
        // skip empty titles and dedup by title to drop repeated ad rows.
        if title.is_empty() || !seen_titles.insert(title.clone()) {
            continue;
        }
        let href = a.value().attr("href").unwrap_or("");
        // Bing hrefs are already absolute; base is irrelevant, we only unescape.
        let url = normalize_redirect_href(href, "");
        if url.is_empty() {
            continue;
        }
        let snippet = first_snippet(container, &snippet_sels)
            .unwrap_or_else(|| container_text_minus_title(&container, &h2_sel));
        out.push(SearchResult {
            title,
            url,
            snippet,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Parse a Sogou results page (www.sogou.com) into `{title, url, snippet}`
/// rows. Results live in `div.vrwrap` (and the `div.rb` variant); titles sit
/// under `h3 a`. Unlike Bing, Sogou wraps result links in a relative redirect
/// (`/link?url=...`) that must be made absolute against `https://www.sogou.com`.
/// Snippets are under `.str_info` / `.str-text-info` / `.fz-mid` / `.space-txt`
/// (falling back to the container text with the `h3` title stripped off the
/// front). Titles often wrap keywords in `<em>` with HTML-comment markers
/// (`<!--red_beg-->...<!--red_end-->`); scraper's `text()` skips comment nodes
/// so the title comes out clean. Pure + obscura-free so it is unit-tested in
/// the default build with a frozen fixture.
pub fn parse_sogou_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let container_sel = Selector::parse("div.vrwrap, div.rb").unwrap();
    let h3a_sel = Selector::parse("h3 a").unwrap();
    let h3_sel = Selector::parse("h3").unwrap();
    let snippet_sels = [
        Selector::parse(".str_info").unwrap(),
        Selector::parse(".str-text-info").unwrap(),
        Selector::parse(".fz-mid").unwrap(),
        Selector::parse(".space-txt").unwrap(),
    ];
    let mut out = Vec::new();
    let mut seen_titles = std::collections::HashSet::new();
    for container in doc.select(&container_sel) {
        let Some(a) = container.select(&h3a_sel).next() else {
            continue;
        };
        let title = normalize_ws(&a.text().collect::<Vec<_>>().join(""));
        // skip empty titles and dedup by title to drop repeated ad rows.
        if title.is_empty() || !seen_titles.insert(title.clone()) {
            continue;
        }
        let href = a.value().attr("href").unwrap_or("");
        // Sogou uses relative /link?url=... redirects; make absolute then unescape.
        let url = normalize_redirect_href(href, "https://www.sogou.com");
        if url.is_empty() {
            continue;
        }
        let snippet = first_snippet(container, &snippet_sels)
            .unwrap_or_else(|| container_text_minus_title(&container, &h3_sel));
        out.push(SearchResult {
            title,
            url,
            snippet,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Dispatcher: parse a search-engine results page into `SearchResult` rows
/// based on the URL's host. Currently recognises Baidu (`baidu.com`),
/// DuckDuckGo (`duckduckgo.com`), Bing (`bing.com`) and Sogou (`sogou.com`);
/// returns an empty `Vec` for any other host (i.e. not a known search page),
/// signalling the caller to fall back to generic readable-text extraction.
pub fn parse_search_results(url: &Url, html: &str, limit: usize) -> Vec<SearchResult> {
    let host = url.host_str().unwrap_or("");
    if host.contains("baidu.com") {
        parse_baidu_results(html, limit)
    } else if host.contains("duckduckgo.com") {
        parse_ddg_results(html, limit)
    } else if host.contains("bing.com") {
        parse_bing_results(html, limit)
    } else if host.contains("sogou.com") {
        parse_sogou_results(html, limit)
    } else {
        Vec::new()
    }
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

    // Compact Baidu SERP fixture: one organic `result.c-container` (title with
    // `<em>` + `&nbsp;`, `.c-abstract` snippet, baidu redirect href) and one
    // one-box `result-op.c-container` (no `.c-abstract` → fallback snippet), plus
    // a `<script>` and `<nav>` that must NOT leak into any parsed field.
    const BAIDU_FIXTURE: &str = r#"<html><head>
<script>var js_noise = "title_script_leak"; var nav_leak = "snippet_script_leak";</script>
</head><body>
<nav>导航文本 nav_noise_leak_here</nav>
<div id="content">
  <div class="result c-container" id="1">
    <h3><a href="http://www.baidu.com/link?url=abc123&amp;wd=foo">2026年&nbsp;<em>国内AI大模型</em></a></h3>
    <div class="c-abstract">Qwen系列与DeepSeek领跑，详细介绍 <span>full</span> 排行榜。</div>
  </div>
  <div class="result-op c-container" id="2">
    <h3><a href="baidu.php?url=http%3A%2F%2Fexample.cn%2Fx">即时工具箱 <em>AI导航</em></a></h3>
    <span class="desc">这是即时摘要 desc_text，非abstract。</span>
  </div>
  <script>more_js_noise = "should_not_leak";</script>
</div></body></html>"#;

    #[test]
    fn parse_baidu_extracts_title_url_snippet() {
        let r = parse_baidu_results(BAIDU_FIXTURE, 8);
        assert_eq!(r.len(), 2, "expected 2 results (organic + one-box)");
        // first title: <em>/<&nbsp;> markup decoded, nbsp → space, no stray chars.
        assert!(r[0].title.contains("2026年"), "title: {}", r[0].title);
        assert!(r[0].title.contains("国内AI大模型"), "title: {}", r[0].title);
        assert!(
            !r[0].title.contains("<"),
            "no markup in title: {}",
            r[0].title
        );
        // first snippet from .c-abstract.
        assert!(
            r[0].snippet.contains("Qwen系列"),
            "snippet: {}",
            r[0].snippet
        );
        // first url: baidu redirect, &amp; unescaped to &.
        assert!(r[0].url.contains("baidu.com/link"), "url: {}", r[0].url);
        assert!(r[0].url.contains("&wd=foo"), "amp unescaped: {}", r[0].url);
        // second (one-box) uses the fallback snippet path.
        assert!(
            r[1].snippet.contains("desc_text"),
            "fallback snippet: {}",
            r[1].snippet
        );
        // NO script/nav noise leaks into any title or snippet.
        for row in &r {
            assert!(!row.title.contains("leak"), "title leak: {}", row.title);
            assert!(
                !row.snippet.contains("nav_leak"),
                "snippet nav leak: {}",
                row.snippet
            );
            assert!(
                !row.snippet.contains("title_script_leak"),
                "snippet script leak: {}",
                row.snippet
            );
            assert!(
                !row.snippet.contains("should_not_leak"),
                "snippet script2 leak: {}",
                row.snippet
            );
        }
        // nbsp must never survive as a stray non-breaking space.
        for row in &r {
            assert!(
                !row.title.contains('\u{a0}'),
                "nbsp in title: {:?}",
                row.title
            );
            assert!(
                !row.snippet.contains('\u{a0}'),
                "nbsp in snippet: {:?}",
                row.snippet
            );
        }
    }

    #[test]
    fn parse_baidu_respects_limit() {
        let r = parse_baidu_results(BAIDU_FIXTURE, 1);
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains("2026年"));
    }

    // Compact Bing SERP fixture: two `li.b_algo` rows (one with the snippet
    // nested under `.b_caption p.b_lineclamp2`, one with a bare `p.b_lineclamp1`)
    // plus a third row whose anchor href is empty (must be skipped), and a
    // `<script>` that must NOT leak into any parsed field.
    const BING_FIXTURE: &str = r#"<!DOCTYPE html><html><body>
<div id="b_results"><li class="b_algo">
  <h2><a href="https://example.com/blog/llm">2026 <em>开源大模型</em>横评排行榜</a></h2>
  <div class="b_caption"><p class="b_lineclamp2">2026年6月14日 · 综合三轮测试的完成度，给出实测排行榜。DeepSeek-V3 重构能力极强。</p></div>
</li>
<li class="b_algo">
  <h2><a href="https://gitee.com/oschina&amp;ref=x">开源中国 - Gitee</a></h2>
  <p class="b_lineclamp1">自2013年上线以来，Gitee服务了1200万开发者。</p>
</li>
<li class="b_algo">
  <h2><a href="">NoHref Skip Me</a></h2>
</li></div>
<script>noise=1</script>
</body></html>"#;

    #[test]
    fn parse_bing_extracts_title_url_snippet() {
        let r = parse_bing_results(BING_FIXTURE, 8);
        // empty-href row is skipped → 2 results.
        assert_eq!(r.len(), 2);
        // first title: <em> markup flattened, no stray chars.
        assert_eq!(r[0].title, "2026 开源大模型横评排行榜");
        assert_eq!(r[0].url, "https://example.com/blog/llm");
        // first snippet from p.b_lineclamp2 (inside .b_caption).
        assert!(r[0].snippet.contains("DeepSeek-V3"));
        // second url: &amp; unescaped to &.
        assert_eq!(r[1].url, "https://gitee.com/oschina&ref=x");
        // NO script noise leaks into any field.
        for row in &r {
            assert!(!row.title.contains("noise"));
            assert!(!row.snippet.contains("noise"));
            assert!(!row.url.contains("noise"));
        }
    }

    #[test]
    fn parse_bing_respects_limit() {
        let r = parse_bing_results(BING_FIXTURE, 1);
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains("开源大模型"));
    }

    // Compact Sogou SERP fixture: two `div.vrwrap` rows (first with `.str_info`
    // snippet and a relative `/link?url=...` redirect; second with a `.fz-mid`
    // snippet and an absolute href containing `&amp;`) plus a third empty-href
    // row (must be skipped), and a `<nav>` that must NOT leak.
    const SOGOU_FIXTURE: &str = r#"<!DOCTYPE html><html><body>
<div class="results">
<div class="vrwrap"><h3 class="vr-title"><a href="/link?url=hedJjaC291ObqPUCEo1zMura">全球<em><!--red_beg-->开源大模型<!--red_end--></em>最新排名Top10</a></h3>
<div class="str_info"><span class="c-color-text">DeepSeek-R1智能体性价比之王，代码与数学推理全球顶尖。</span></div></div>
<div class="vrwrap"><h3 class="vr-title"><a href="https://mp.weixin.qq.com/s?src=11&amp;t=1">大模型排行榜&nbsp;今日头条</a></h3>
<div class="fz-mid">文心5.1搜索能力全球第四。</div></div>
<div class="vrwrap"><h3 class="vr-title"><a href="">EmptyHref Skip</a></h3></div>
</div>
<nav>nav links</nav>
</body></html>"#;

    #[test]
    fn parse_sogou_extracts_title_url_snippet() {
        let r = parse_sogou_results(SOGOU_FIXTURE, 8);
        // empty-href row is skipped → 2 results.
        assert_eq!(r.len(), 2);
        // first title: comment nodes + <em> flattened, no stray chars.
        assert_eq!(r[0].title, "全球开源大模型最新排名Top10");
        // relative /link?url=... made absolute.
        assert_eq!(
            r[0].url,
            "https://www.sogou.com/link?url=hedJjaC291ObqPUCEo1zMura"
        );
        // first snippet from .str_info; second snippet from .fz-mid.
        assert!(r[0].snippet.contains("DeepSeek-R1"));
        assert!(r[1].snippet.contains("文心5.1"));
        // second url: &amp; unescaped to &.
        assert!(r[1].url.contains('&'));
        assert!(!r[1].url.contains("&amp;"));
        // NO nav noise leaks, and nbsp must never survive as \u{a0}.
        for row in &r {
            assert!(!row.title.contains("nav"));
            assert!(!row.snippet.contains("nav"));
            assert!(!row.title.contains('\u{a0}'));
        }
    }

    #[test]
    fn parse_search_results_dispatches_by_host() {
        // baidu host → structured results.
        let u = normalize_url("https://www.baidu.com/s?wd=x").unwrap();
        let r = parse_search_results(&u, BAIDU_FIXTURE, 12);
        assert!(!r.is_empty(), "baidu host should produce results");
        // non-search host → empty (fall back to readable text).
        let u2 = normalize_url("https://example.com/").unwrap();
        let r2 = parse_search_results(&u2, BAIDU_FIXTURE, 12);
        assert!(r2.is_empty(), "non-search host should yield no results");
        // ddg host → ddg parser.
        let u3 = normalize_url("https://html.duckduckgo.com/html/").unwrap();
        let r3 = parse_search_results(&u3, DDG_FIXTURE, 12);
        assert!(!r3.is_empty(), "ddg host should produce results");
        // bing host → bing parser.
        let u4 = Url::parse("https://cn.bing.com/search?q=x").unwrap();
        let r4 = parse_search_results(&u4, BING_FIXTURE, 12);
        assert!(!r4.is_empty(), "bing host should produce results");
        // sogou host → sogou parser.
        let u5 = Url::parse("https://www.sogou.com/web?query=x").unwrap();
        let r5 = parse_search_results(&u5, SOGOU_FIXTURE, 12);
        assert!(!r5.is_empty(), "sogou host should produce results");
    }
}
