//! Search-engine results page (SERP) parsing: extract `{title, url, snippet}`
//! rows from the HTML of a search page (Baidu / Bing / Sogou / DuckDuckGo /
//! Google) or a site search (GitHub repositories, HuggingFace models). Each
//! engine has a dedicated CSS-selector parser in [`engines`];
//! [`parse_search_results`] dispatches by URL host. All parsers are pure and
//! feature-independent so they compile and are unit-tested in the default
//! (no-`browser`) build.

use url::Url;

#[path = "serp_engines.rs"]
mod engines;

pub use engines::*;

/// A single search result row parsed from a search-engine or site results page.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Normalize whitespace in extracted text: replace non-breaking spaces
/// (`\u{a0}`, what `&nbsp;` decodes to under scraper's `text()`) with a normal
/// space, then collapse all runs of whitespace into single spaces (and trim).
fn normalize_ws(s: &str) -> String {
    s.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Unescape `&amp;` -> `&` in a Baidu redirect href and trim surrounding space.
fn normalize_baidu_href(href: &str) -> String {
    href.trim().replace("&amp;", "&")
}

/// Make a possibly-relative href absolute against `base`, then unescape
/// `&amp;` -> `&`. Empty stays empty.
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
/// normalized; returns `None` when none match.
fn first_snippet(scope: scraper::ElementRef, sels: &[scraper::Selector]) -> Option<String> {
    sels.iter()
        .find_map(|sel| scope.select(sel).next())
        .map(|el| normalize_ws(&el.text().collect::<Vec<_>>().join(" ")))
}

/// Fallback snippet: the container's full text with the leading title block
/// stripped off the front.
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

/// Dispatcher: parse a search-engine or site results page into `SearchResult`
/// rows based on the URL's host. Recognises Baidu, DuckDuckGo, Bing, Sogou,
/// Google, GitHub and HuggingFace; returns an empty `Vec` for unknown hosts,
/// signalling the caller to fall back to generic readable-text extraction.
pub fn parse_search_results(url: &Url, html: &str, limit: usize) -> Vec<SearchResult> {
    let host = url.host_str().unwrap_or("");
    if host.contains("baidu.com") {
        engines::parse_baidu_results(html, limit)
    } else if host.contains("duckduckgo.com") {
        engines::parse_ddg_results(html, limit)
    } else if host.contains("bing.com") {
        engines::parse_bing_results(html, limit)
    } else if host.contains("sogou.com") {
        engines::parse_sogou_results(html, limit)
    } else if host.contains("google.") {
        engines::parse_google_results(html, limit)
    } else if host.contains("github.com") {
        engines::parse_github_results(html, limit)
    } else if host.contains("huggingface.co") {
        engines::parse_hf_results(html, limit)
    } else {
        Vec::new()
    }
}

#[cfg(test)]
#[path = "serp_tests.rs"]
mod tests;
