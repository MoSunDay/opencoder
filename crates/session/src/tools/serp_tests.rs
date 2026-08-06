use super::*;
use url::Url;

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
        let u = Url::parse("https://www.baidu.com/s?wd=x").unwrap();
        let r = parse_search_results(&u, BAIDU_FIXTURE, 12);
        assert!(!r.is_empty(), "baidu host should produce results");
        // non-search host → empty (fall back to readable text).
        let u2 = Url::parse("https://example.com/").unwrap();
        let r2 = parse_search_results(&u2, BAIDU_FIXTURE, 12);
        assert!(r2.is_empty(), "non-search host should yield no results");
        // ddg host → ddg parser.
        let u3 = Url::parse("https://html.duckduckgo.com/html/").unwrap();
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

    const GOOGLE_FIXTURE: &str = r#"<html><body>
<div id="search">
  <div class="g">
    <div class="tF2Cxc">
      <div><a href="https://www.rust-lang.org/"><h3>The Rust Programming Language</h3></a></div>
      <div class="VwiC3b">A language empowering everyone to build reliable software.</div>
    </div>
  </div>
  <div class="g">
    <a href="/url?q=https://doc.rust-lang.org/book/&sa=U"><h3>The Rust Book</h3></a>
    <div class="VwiC3b">The official guide to Rust.</div>
  </div>
  <div class="g">
    <a href="/search?q=rust+other&safe=active"><h3>Should be skipped</h3></a>
  </div>
</div></body></html>"#;

    #[test]
    fn parse_google_extracts_title_url_snippet() {
        let r = parse_google_results(GOOGLE_FIXTURE, 8);
        assert_eq!(r.len(), 2, "should get 2 results (3rd is /search? skipped)");
        assert_eq!(r[0].title, "The Rust Programming Language");
        assert_eq!(r[0].url, "https://www.rust-lang.org/");
        assert!(r[0].snippet.contains("empowering"));
        // Google redirect /url?q= unwrapped.
        assert_eq!(r[1].url, "https://doc.rust-lang.org/book/");
        assert_eq!(r[1].title, "The Rust Book");
    }

    #[test]
    fn parse_google_respects_limit() {
        let r = parse_google_results(GOOGLE_FIXTURE, 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn parse_google_handles_empty() {
        assert!(parse_google_results("<html></html>", 5).is_empty());
    }

    const GITHUB_FIXTURE: &str = r#"<html><body>
<div>
  <a href="/rust-lang/rust">rust-lang/rust</a>
  <p>Empowering everyone to build reliable and efficient software.</p>
</div>
<div>
  <a href="/tokio-rs/tokio">tokio-rs/tokio</a>
  <p>A runtime for writing reliable asynchronous applications.</p>
</div>
<a href="/features">Features</a>
<a href="/pricing">Pricing</a>
</body></html>"#;

    #[test]
    fn parse_github_extracts_repos() {
        let r = parse_github_results(GITHUB_FIXTURE, 8);
        assert_eq!(r.len(), 2, "should get 2 repos (features/pricing skipped)");
        assert_eq!(r[0].url, "https://github.com/rust-lang/rust");
        assert_eq!(r[0].title, "rust-lang/rust");
        assert!(r[0].snippet.contains("Empowering"));
        assert_eq!(r[1].url, "https://github.com/tokio-rs/tokio");
    }

    #[test]
    fn parse_github_dedups() {
        let html = r#"<a href="/a/b">a/b</a><a href="/a/b">a/b again</a>"#;
        let r = parse_github_results(html, 5);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn parse_github_handles_empty() {
        assert!(parse_github_results("<html></html>", 5).is_empty());
    }

    const HF_FIXTURE: &str = r#"<html><body>
<div>
  <a href="/deepseek-ai/DeepSeek-R1">DeepSeek R1</a>
  <p>DeepSeek-R1, an enhanced reasoning model.</p>
</div>
<div>
  <a href="/Qwen/Qwen2.5-72B">Qwen2.5-72B</a>
  <p>Large language model by Qwen team.</p>
</div>
<a href="/datasets">Datasets</a>
</body></html>"#;

    #[test]
    fn parse_hf_extracts_models() {
        let r = parse_hf_results(HF_FIXTURE, 8);
        assert_eq!(r.len(), 2, "should get 2 models (datasets skipped)");
        assert_eq!(r[0].url, "https://huggingface.co/deepseek-ai/DeepSeek-R1");
        assert_eq!(r[0].title, "DeepSeek R1");
        assert!(r[0].snippet.contains("enhanced reasoning"));
        assert_eq!(r[1].url, "https://huggingface.co/Qwen/Qwen2.5-72B");
    }

    #[test]
    fn parse_hf_handles_empty() {
        assert!(parse_hf_results("<html></html>", 5).is_empty());
    }

    #[test]
    fn parse_search_results_dispatches_all_engines() {
        // google host → google parser.
        let ug = Url::parse("https://www.google.com/search?q=x").unwrap();
        assert!(!parse_search_results(&ug, GOOGLE_FIXTURE, 12).is_empty(),
            "google host should produce results");
        // github host → github parser.
        let ugh = Url::parse("https://github.com/search?q=x&type=repositories").unwrap();
        assert!(!parse_search_results(&ugh, GITHUB_FIXTURE, 12).is_empty(),
            "github host should produce results");
        // huggingface host → hf parser.
        let uhf = Url::parse("https://huggingface.co/models?search=x").unwrap();
        assert!(!parse_search_results(&uhf, HF_FIXTURE, 12).is_empty(),
            "huggingface host should produce results");
    }
