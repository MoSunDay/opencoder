use super::*;

/// Build a page with a `Fallback Title` `<title>` and run `extract_article`,
/// asserting title / content-substrings / content-absences in one call.
fn check(url: &str, body: &str, want_title: &str, want_contains: &[&str], want_absent: &[&str]) {
    let html =
        format!("<html><head><title>Fallback Title</title></head><body>{body}</body></html>");
    let a = extract_article(&html, &Url::parse(url).unwrap());
    if !want_title.is_empty() {
        assert_eq!(a.title, want_title, "title mismatch for {url}");
    }
    for w in want_contains {
        assert!(
            a.content.contains(w),
            "content missing {w:?} for {url}: {}",
            a.content
        );
    }
    for w in want_absent {
        assert!(
            !a.content.contains(w),
            "content should not contain {w:?} for {url}: {}",
            a.content
        );
    }
}

#[test]
fn zhihu_extracts_title_and_content() {
    check("https://www.zhihu.com/question/1",
        "<h1 class=\"QuestionHeader-title\">如何评价 Rust 2024 edition？</h1><div class=\"Post-RichTextContainer\"><p>正文第一段。</p><p>正文第二段。</p></div>",
        "如何评价 Rust 2024 edition？", &["正文第一段。", "正文第二段。"], &[]);
    // zhuanlan subdomain shares the zhihu profile
    check("https://zhuanlan.zhihu.com/p/42",
        "<h1 class=\"Post-Title\">专栏文章</h1><div class=\"Post-RichTextContainer\"><p>内容。</p></div>",
        "专栏文章", &["内容。"], &[]);
}

#[test]
fn csdn_strips_noise_blocks() {
    check("https://blog.csdn.net/a/b/article/details/1",
        "<h1 class=\"title-article\">CSDN 标题</h1><div id=\"article_content\"><p>正文。</p><div class=\"hide-article-box\">阅读全文</div><div class=\"more-toolbox\">工具条广告</div></div>",
        "CSDN 标题", &["正文。"], &["阅读全文", "工具条广告"]);
}

#[test]
fn juejin_extracts_markdown_body() {
    check("https://juejin.cn/post/1",
        "<h1 class=\"article-title\">掘金文章</h1><article class=\"article-content\"><p>第一段</p><p>第二段</p></article>",
        "掘金文章", &["第一段"], &[]);
}

#[test]
fn weixin_extracts_author_and_date() {
    let html = "<html><head><title>t</title></head><body><h1 id=\"activity-name\">公众号标题</h1><span id=\"js_name\">作者名</span><em id=\"publish_time\">2026-08-01</em><div id=\"js_content\"><p>公众号正文。</p></div></body></html>".to_string();
    let a = extract_article(
        &html,
        &Url::parse("https://mp.weixin.qq.com/s/abc").unwrap(),
    );
    assert_eq!(
        (a.title.as_str(), a.author.as_str(), a.date.as_str()),
        ("公众号标题", "作者名", "2026-08-01")
    );
    assert!(a.content.contains("公众号正文。"));
}

#[test]
fn wikipedia_strips_edit_links() {
    check("https://zh.wikipedia.org/wiki/Rust",
        "<h1 id=\"firstHeading\">Rust</h1><div id=\"mw-content-text\"><div class=\"mw-parser-output\"><p>正文内容。</p><span class=\"mw-editsection\">[编辑]</span></div></div>",
        "Rust", &["正文内容。"], &["编辑"]);
}

#[test]
fn github_uses_og_title() {
    check("https://github.com/rust-lang/rust/blob/master/README.md",
        "<meta property=\"og:title\" content=\"rust-lang/rust: README\"><article class=\"markdown-body\"><h1>Rust</h1><p>系统编程语言。</p></article>",
        "rust-lang/rust: README", &["系统编程语言。"], &[]);
}

#[test]
fn stackoverflow_extracts_question() {
    check("https://stackoverflow.com/questions/1/x",
        "<div id=\"question-header\"><h1>Rust borrow checker question</h1></div><div class=\"s-prose\"><p>详细问题描述。</p></div>",
        "Rust borrow checker question", &["详细问题描述。"], &[]);
}

#[test]
fn unknown_host_falls_back_to_generic_extraction() {
    let html = "<html><head><title>Fallback Title</title></head><body><main><article><h1>通用标题</h1><p>通用正文。</p></article></main></body></html>".to_string();
    let a = extract_article(&html, &Url::parse("https://example.com/blog/post").unwrap());
    assert_eq!(a.title, "Fallback Title");
    assert!(a.content.contains("通用正文。"));
}

#[test]
fn format_article_renders_markdown() {
    let a = ExtractedArticle {
        title: "标题".into(),
        author: "作者".into(),
        date: "2026-08-01".into(),
        url: "https://example.com/x".into(),
        content: "正文。".into(),
    };
    let out = format_article(&a);
    assert!(out.starts_with("# 标题\n"));
    assert!(out.contains("- url: https://example.com/x"));
    assert!(out.contains("- author: 作者"));
    assert!(out.contains("- date: 2026-08-01"));
    assert!(out.ends_with("正文。"));
}

#[tokio::test]
async fn web_extract_tool_executes() {
    use opencoder_core::ToolContext;
    let tool = WebExtractTool;
    let ctx = ToolContext {
        session_id: "s".into(),
        message_id: "m".into(),
        agent: "act".into(),
        working_dir: std::env::temp_dir(),
        max_output: 4096,
        proxy: None,
    };
    let out = tool
        .execute(
            serde_json::json!({"html": "<html><body><h1 class=\"title-article\">T</h1><div id=\"article_content\"><p>Body</p></div></body></html>", "url": "https://blog.csdn.net/x/y/article/details/1"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!out.is_error);
    assert!(out.content.contains("T"), "{}", out.content);
    assert!(out.content.contains("Body"));
    let err = tool
        .execute(serde_json::json!({"html": ""}), &ctx)
        .await
        .unwrap();
    assert!(err.is_error);
}
