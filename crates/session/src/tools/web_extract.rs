//! Structured article extraction from rendered HTML via a site-profile
//! registry (`SITE_PROFILES`). [`extract_article`] matches the URL host
//! against known sites (zhihu / csdn / juejin / weixin / cnblogs / wikipedia /
//! github / 36kr / stackoverflow), pulls title/author/date/content with fixed
//! CSS selectors and strips noise blocks; unknown hosts fall back to the
//! generic readable-text extractor from `super::web_read`. Pure +
//! feature-independent, so it compiles and is unit-tested in the default
//! (no-`browser`) build.

use scraper::{Html, Selector};
use url::Url;

use super::web_read;

/// A fixed analysis recipe for one site (or family sharing a host suffix).
/// Content selectors are tried in priority order; everything else is optional
/// and falls back to document-level extraction when absent.
pub struct SiteProfile {
    pub host_matches: &'static [&'static str],
    pub title_sel: Option<&'static str>,
    pub content_sels: &'static [&'static str],
    pub noise_sels: &'static [&'static str],
    pub author_sel: Option<&'static str>,
    pub date_sel: Option<&'static str>,
}

macro_rules! profile {
    ($host:expr, $title:expr, $content:expr, $noise:expr, $author:expr, $date:expr) => {
        SiteProfile {
            host_matches: $host,
            title_sel: $title,
            content_sels: $content,
            noise_sels: $noise,
            author_sel: $author,
            date_sel: $date,
        }
    };
}

/// Registry of fixed site analyses. Host matching is a substring check, so
/// `zhihu.com` also covers `zhuanlan.zhihu.com`; keep in sync with the
/// chrome-headless skill doc.
pub static SITE_PROFILES: &[SiteProfile] = &[
    profile!(
        &["zhihu.com"],
        Some("h1.QuestionHeader-title, .Post-Title"),
        &[".Post-RichTextContainer", ".RichContent-inner"],
        &[],
        None,
        None
    ),
    profile!(
        &["csdn.net"],
        Some("h1.title-article"),
        &["#article_content", "article"],
        &[".hide-article-box", ".more-toolbox"],
        None,
        None
    ),
    profile!(
        &["juejin.cn"],
        Some("h1.article-title"),
        &["article.article-content", ".markdown-body"],
        &[],
        None,
        None
    ),
    profile!(
        &["weixin.qq.com"],
        Some("#activity-name"),
        &["#js_content"],
        &[],
        Some("#js_name"),
        Some("#publish_time")
    ),
    profile!(
        &["cnblogs.com"],
        Some("#cb_post_title_url, h1.postTitle"),
        &["#cnblogs_post_body"],
        &[],
        None,
        None
    ),
    profile!(
        &["wikipedia.org"],
        Some("#firstHeading"),
        &["#mw-content-text .mw-parser-output"],
        &[".mw-editsection"],
        None,
        None
    ),
    // GitHub pages carry no stable article heading; rely on og:title.
    profile!(
        &["github.com"],
        None,
        &["article.markdown-body", "#readme .markdown-body"],
        &[],
        None,
        None
    ),
    profile!(
        &["36kr.com"],
        Some("h1.article-title"),
        &[".article-content"],
        &[],
        None,
        None
    ),
    profile!(
        &["stackoverflow.com"],
        Some("#question-header h1"),
        &[".s-prose", ".post-content"],
        &[],
        None,
        None
    ),
];

/// A distilled article: metadata plus cleaned body text.
#[derive(Debug, Clone, Default)]
pub struct ExtractedArticle {
    pub title: String,
    pub author: String,
    pub date: String,
    pub url: String,
    pub content: String,
}

/// Collapse whitespace (incl. `\u{a0}` from `&nbsp;`) into single spaces.
fn norm_ws(s: &str) -> String {
    s.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collapse runs of blank lines to a single one, trimming the tail.
fn collapse_blank(s: &str) -> String {
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

/// First matching element's normalized text for `sel`, or `None`.
fn first_text(doc: &Html, sel: &str) -> Option<String> {
    let sel = Selector::parse(sel).ok()?;
    doc.select(&sel)
        .next()
        .map(|el| norm_ws(&el.text().collect::<String>()))
        .filter(|t| !t.is_empty())
}

/// Document-level title fallback: `og:title` meta, then `<title>`.
fn page_title(doc: &Html) -> Option<String> {
    if let Some(el) = Selector::parse("meta[property='og:title'], meta[name='og:title']")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
    {
        let t = norm_ws(el.attr("content").unwrap_or(""));
        if !t.is_empty() {
            return Some(t);
        }
    }
    Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| norm_ws(&el.text().collect::<String>()))
        .filter(|t| !t.is_empty())
}

/// Re-parse the content inner HTML and detach every element matching a noise
/// selector, so ads / fold boxes / edit links never reach the text output.
fn strip_noise(content_html: &str, noise_sels: &[&'static str]) -> String {
    if noise_sels.is_empty() {
        return content_html.to_string();
    }
    let mut frag = Html::parse_fragment(content_html);
    let mut ids = Vec::new();
    for sel_str in noise_sels {
        if let Ok(sel) = Selector::parse(sel_str) {
            ids.extend(frag.select(&sel).map(|el| el.id()));
        }
    }
    for id in ids {
        if let Some(mut node) = frag.tree.get_mut(id) {
            node.detach();
        }
    }
    frag.html()
}

/// First content selector that yields non-empty text, noise-stripped and
/// converted to readable plain text.
fn extract_content(doc: &Html, sels: &[&'static str], noise: &[&'static str]) -> String {
    for sel_str in sels {
        let Ok(sel) = Selector::parse(sel_str) else {
            continue;
        };
        let Some(el) = doc.select(&sel).next() else {
            continue;
        };
        let html = strip_noise(&el.inner_html(), noise);
        let raw = html2text::from_read(html.as_bytes(), 100).unwrap_or_default();
        let text = collapse_blank(raw.trim());
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

/// Extract a structured article from rendered HTML, picking the site profile
/// by URL host. Unknown hosts fall back to the generic readable-text
/// extractor; the title then comes from `og:title`/`<title>`.
pub fn extract_article(html: &str, url: &Url) -> ExtractedArticle {
    let host = url.host_str().unwrap_or("");
    let profile = SITE_PROFILES
        .iter()
        .find(|p| p.host_matches.iter().any(|h| host.contains(h)));
    let doc = Html::parse_document(html);
    let (title, author, date, content) = match profile {
        Some(p) => (
            p.title_sel
                .and_then(|s| first_text(&doc, s))
                .or_else(|| page_title(&doc))
                .unwrap_or_default(),
            p.author_sel
                .and_then(|s| first_text(&doc, s))
                .unwrap_or_default(),
            p.date_sel
                .and_then(|s| first_text(&doc, s))
                .unwrap_or_default(),
            extract_content(&doc, p.content_sels, p.noise_sels),
        ),
        None => (
            page_title(&doc).unwrap_or_default(),
            String::new(),
            String::new(),
            web_read::extract_readable_text(html),
        ),
    };
    ExtractedArticle {
        title,
        author,
        date,
        url: url.to_string(),
        content,
    }
}

/// Render an article as markdown: `# title`, a metadata bullet block (url /
/// optional author / optional date), then the body.
pub fn format_article(a: &ExtractedArticle) -> String {
    let heading = if a.title.is_empty() {
        format!("# {}", a.url)
    } else {
        format!("# {}", a.title)
    };
    let mut out = heading;
    out.push_str(&format!("\n- url: {}", a.url));
    if !a.author.is_empty() {
        out.push_str(&format!("\n- author: {}", a.author));
    }
    if !a.date.is_empty() {
        out.push_str(&format!("\n- date: {}", a.date));
    }
    out.push_str("\n\n");
    out.push_str(&a.content);
    out
}

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct WebExtractTool;

#[async_trait]
impl Tool for WebExtractTool {
    fn name(&self) -> &str {
        "web_extract"
    }
    fn description(&self) -> &str {
        "Extract a structured article (title/author/date/body) from raw rendered HTML. \
         Site-aware profiles for zhihu, csdn, juejin, weixin, cnblogs, wikipedia, github, \
         36kr, stackoverflow; generic fallback otherwise. Pair with \
         chrome_headless(action=\"html\")."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "html".into(),
            json::prop_str("Raw rendered HTML (from chrome_headless action=\"html\")."),
        );
        props.insert(
            "url".into(),
            json::prop_str("The page URL, used to pick the site profile."),
        );
        json::object_schema(Value::Object(props), &["html", "url"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let html = input.get("html").and_then(|v| v.as_str()).unwrap_or("");
        let url_str = input.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if html.is_empty() || url_str.is_empty() {
            return Ok(ToolOutput::err("Both 'html' and 'url' are required."));
        }
        let url = match Url::parse(url_str) {
            Ok(u) => u,
            Err(e) => return Ok(ToolOutput::err(format!("Invalid url: {e}"))),
        };
        let article = extract_article(html, &url);
        Ok(truncate_output(format_article(&article), ctx.max_output))
    }
}

#[cfg(test)]
#[path = "web_extract_tests.rs"]
mod tests;
