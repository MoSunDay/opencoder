//! Per-engine SERP parsers. Each function takes raw HTML + a result limit and
//! returns `Vec<SearchResult>`. Shared helpers (`normalize_ws`,
//! `normalize_redirect_href`, `first_snippet`, `container_text_minus_title`)
//! are inherited from the parent [`super`] module.

use super::*;

/// Parse DuckDuckGo results. Unwraps the `uddg` redirect param to the real URL.
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

/// Unwrap a DDG `//duckduckgo.com/l/?uddg=<url>` redirect to the real target.
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

/// Parse Baidu results. Containers are `div.c-container`; titles from `h3 a`,
/// snippets from `.c-abstract`. Redirect hrefs kept verbatim.
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

/// Parse Bing results. Containers are `li.b_algo`; titles from `h2 a` (direct URLs), snippets from `p.b_lineclamp*` or `.b_caption p`.
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

/// Parse Sogou results. Containers are `div.vrwrap`/`div.rb`; titles from `h3 a`, snippets from `.str_info`/`.fz-mid` etc. Redirect links made absolute.
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

/// Parse Google results. Containers are `div.g`; titles from `h3`, URLs from wrapping `a` (unwraps `/url?q=` redirects). Snippets via multi-level fallback. May return empty on CAPTCHA — fall back to Bing/DDG.
pub fn parse_google_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let container_sel = Selector::parse("div.g, div.Gx5Zad, div.tF2Cxc").unwrap();
    let h3_sel = Selector::parse("h3").unwrap();
    let a_sel = Selector::parse("a").unwrap();
    let snippet_sels = [
        Selector::parse("div[data-sncf]").unwrap(),
        Selector::parse("div.VwiC3b").unwrap(),
        Selector::parse("span.aCOSRe").unwrap(),
        Selector::parse("div[data-snc] > div").unwrap(),
        Selector::parse("div.IsZvec").unwrap(),
    ];
    let mut out = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();
    for container in doc.select(&container_sel) {
        let Some(h3) = container.select(&h3_sel).next() else {
            continue;
        };
        let title = normalize_ws(&h3.text().collect::<Vec<_>>().join(""));
        if title.is_empty() {
            continue;
        }
        // The h3 is typically wrapped inside the result anchor.
        let Some(a) = container.select(&a_sel).next() else {
            continue;
        };
        let href = a.value().attr("href").unwrap_or("");
        // Skip Google internal / relative search links.
        if href.is_empty() || href.starts_with("/search") {
            continue;
        }
        // Unwrap Google redirect: /url?q=<real>&sa=...
        let url = if let Some(q) = href.strip_prefix("/url?") {
            let parsed = Url::parse(&format!("https://www.google.com/url?{q}")).ok();
            parsed
                .and_then(|u| {
                    u.query_pairs()
                        .find(|(k, _)| k == "q")
                        .map(|(_, v)| v.to_string())
                })
                .unwrap_or_default()
        } else if href.contains("google.com/search") {
            continue;
        } else {
            href.to_string()
        };
        if url.is_empty() || !seen_urls.insert(url.clone()) {
            continue;
        }
        let snippet = first_snippet(container, &snippet_sels).unwrap_or_default();
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

/// Parse GitHub repo search. Extracts `/{owner}/{repo}` links (2 segments, no dots). SSR page; titles from anchor text, snippets from sibling `<p>`.
pub fn parse_github_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let a_sel = Selector::parse("a[href]").unwrap();
    let p_sel = Selector::parse("p").unwrap();
    let skip_prefixes = [
        "settings", "notifications", "login", "signup", "explore", "topics",
        "trending", "collections", "events", "sponsors", "search", "marketplace",
        "pricing", "features", "security", "team", "enterprise", "about",
        "organizations", "new", "customer-stories", "blog",
    ];
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for a in doc.select(&a_sel) {
        let href = a.value().attr("href").unwrap_or("");
        let path = href
            .strip_prefix("https://github.com")
            .or_else(|| href.strip_prefix("http://github.com"))
            .unwrap_or(href);
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segments.len() != 2
            || segments
                .iter()
                .any(|s| s.is_empty() || s.contains('.'))
        {
            continue;
        }
        if skip_prefixes.contains(&segments[0]) {
            continue;
        }
        let repo_url = format!("https://github.com/{}/{}", segments[0], segments[1]);
        if !seen.insert(repo_url.clone()) {
            continue;
        }
        let title_text = normalize_ws(&a.text().collect::<Vec<_>>().join(""));
        let title = if title_text.is_empty() {
            format!("{}/{}", segments[0], segments[1])
        } else {
            title_text
        };
        // Try to find a description paragraph in the same parent container.
        let snippet = a
            .ancestors()
            .find_map(|anc| {
                scraper::ElementRef::wrap(anc)
                    .and_then(|el| el.select(&p_sel).next())
                    .map(|p| normalize_ws(&p.text().collect::<Vec<_>>().join("")))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url: repo_url,
            snippet,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Parse HuggingFace model search. Extracts `/{org}/{model}` links from SSR cards. Single-segment paths (`/datasets`, `/spaces`) excluded.
pub fn parse_hf_results(html: &str, limit: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let a_sel = Selector::parse("a[href]").unwrap();
    let p_sel = Selector::parse("p").unwrap();
    let skip_segments = [
        "datasets", "spaces", "models", "docs", "settings", "login", "signup",
        "join", "pricing", "enterprise", "solutions", "inference-endpoints",
        "autocomplete", "logout", "notification", "profile",
    ];
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for a in doc.select(&a_sel) {
        let href = a.value().attr("href").unwrap_or("");
        let path = href
            .strip_prefix("https://huggingface.co")
            .or_else(|| href.strip_prefix("http://huggingface.co"))
            .unwrap_or(href);
        // Model paths: /{org}/{model} (2+ segments, no dots in first segment)
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if segments.len() < 2 || segments[0].is_empty() || segments[0].contains('.') {
            continue;
        }
        if skip_segments.contains(&segments[0]) {
            continue;
        }
        let model_url = format!(
            "https://huggingface.co/{}/{}",
            segments[0], segments[1]
        );
        if !seen.insert(model_url.clone()) {
            continue;
        }
        let title_text = normalize_ws(&a.text().collect::<Vec<_>>().join(""));
        let title = if title_text.is_empty() {
            format!("{}/{}", segments[0], segments[1])
        } else {
            title_text
        };
        let snippet = a
            .ancestors()
            .find_map(|anc| {
                scraper::ElementRef::wrap(anc)
                    .and_then(|el| el.select(&p_sel).next())
                    .map(|p| normalize_ws(&p.text().collect::<Vec<_>>().join("")))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url: model_url,
            snippet,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}
