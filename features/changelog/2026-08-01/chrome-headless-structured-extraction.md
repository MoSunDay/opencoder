# chrome_headless 结构化提炼：web_extract + research + resolve/html + 多引擎搜索

## Summary

把 `chrome_headless` 从「能抓 HTML」升级为「能检索、能提炼」的 deepresearch
工具链：新增 `web_extract`（站点结构化提炼，9 个固定 site profile + 通用回退）、
`research`（多引擎搜索 → 去重合并 → 逐结果渲染 → 站点提炼 → 报告落盘的单工具管线）；
`chrome_headless` 新增 `resolve`（渲染解百度/搜狗跳转链接取真实 URL）与 `html`
（原始 DOM 输出）动作，并默认携带真实 Chrome UA + `AutomationControlled` 反检测；
`web_search`（browser feature）从仅 DDG 扩展为四引擎分发。

## Changes

### `crates/session/src/tools/web_extract.rs`（新建）
- `SiteProfile` 注册表 + `SITE_PROFILES`：知乎/CSDN/掘金/公众号/博客园/Wikipedia/
  GitHub/36kr/StackOverflow 固定选择器（标题/正文/作者/日期 + 噪声剔除），
  GitHub 走 `og:title`。
- `extract_article(html, url) -> ExtractedArticle`：命中 profile 用 scraper 抽取、
  ego_tree 节点 detach 剔除噪声块、`html2text` 转文本；未命中回退
  `web_read::extract_readable_text`（标题取 `og:title`/`<title>`）。
- `format_article`：markdown 渲染（`# 标题` / 元信息行 / 正文）。
- `WebExtractTool`（非 feature-gated，无新依赖，scraper/html2text/url 复用）。

### `crates/session/src/tools/chrome_headless.rs`（355 → 560 行）
- 共享 runner `dump_dom(url, wait_ms, ua)`：fetch/resolve/html 复用，附带
  per-call `--user-data-dir`（容器内 snap chromium 必需 + 避免并发 profile 锁）。
- 新增 `chrome_base_args`：默认真实 UA（Chrome/126.0）、
  `--disable-blink-features=AutomationControlled`、`--lang=zh-CN`，可被输入 `ua` 覆盖。
- 新动作 **resolve**：渲染跳转 URL（`is_redirect_url` 识别 baidu.com/sogou.com
  `/link?url=`），`extract_final_url` 从 `<link rel="canonical">`/`og:url` 取真实
  URL，输出 `# Resolved URL` + 标题 + 600 字符摘要。
- 新动作 **html**：返回原始渲染 DOM（经 `truncate_output` 截断）。
- `execute` 分发与参数 schema（enum 增加 resolve/html、新增 `ua` 参数）同步更新。

### `crates/session/src/tools/research.rs`（新建）
- 纯函数：`serp_url`（bing/baidu/sogou/ddg 四引擎 URL 构建，未知引擎回退 bing）、
  `merge_results`（跨引擎按 title+host 去重、按引擎优先级保留）、`slugify`、
  `build_report`（`# Research: <query>` + 逐来源标题/真实 URL/正文节选 + 原始结果清单）。
- `ResearchTool`：逐引擎 chrome 渲染 SERP → `parse_search_results` → 合并 →
  逐结果 `dump_dom`（跳转链接直接渲染）+ `extract_final_url` + `extract_article`
  → 报告写入 `<working_dir>/.research/<slug>-<ts>.md`，返回路径 + 各来源摘要。
  引擎无结果自动跳过，百度验证码不会拖垮整条管线。

### `crates/session/src/tools/web_search.rs`（browser feature）
- 新增 `engine` 参数（bing/baidu/sogou/ddg，默认 ddg），URL 构建复用
  `research::serp_url`，解析统一走 `web_read::parse_search_results`（按 host 分发），
  wait selector 按引擎区分（`.result`/`li.b_algo`/`div.c-container`/`div.vrwrap`）。

### `crates/session/src/tools/mod.rs`
- 注册 `web_extract`、`research`（均非 feature-gated，无新外部依赖）。

### `crates/core/assets/skills/chrome-headless/SKILL.md`
- 固化 deepresearch 工作流：fetch 搜 SERP → resolve 解跳转 → html + web_extract
  提炼 → research 汇总；写明 9 个内置站点分析方式与引擎降级策略。

## Test checklist

### Unit tests（`cargo test -p opencoder-session --lib`）
- `web_extract::tests::zhihu_extracts_title_and_content`（含 zhuanlan 子域命中）
- `web_extract::tests::csdn_strips_noise_blocks`（hide-article-box/more-toolbox 剔除）
- `web_extract::tests::juejin_extracts_markdown_body`
- `web_extract::tests::weixin_extracts_author_and_date`
- `web_extract::tests::wikipedia_strips_edit_links`（mw-editsection 剔除）
- `web_extract::tests::github_uses_og_title`
- `web_extract::tests::stackoverflow_extracts_question`
- `web_extract::tests::unknown_host_falls_back_to_generic_extraction`
- `web_extract::tests::format_article_renders_markdown`
- `web_extract::tests::web_extract_tool_executes`（Tool 实现契约：参数缺失报错 + CSDN HTML 出文）
- `chrome_headless::tests::base_args_use_real_ua_and_anti_detection`
- `chrome_headless::tests::parameters_schema_advertises_all_actions`（action enum 含 fetch/resolve/html/screenshot + required [action,url]）
- `chrome_headless::tests::redirect_urls_are_recognised`（百度/搜狗 link?url 识别）
- `chrome_headless::tests::extract_final_url_prefers_canonical`
- `chrome_headless::tests::extract_final_url_falls_back_to_og_url`
- `chrome_headless::tests::extract_final_url_resolves_relative_canonical`
- `chrome_headless::tests::extract_final_url_returns_fallback_when_absent`
- `research::tests::serp_url_builds_per_engine_urls`
- `research::tests::merge_results_dedups_by_title_and_host`
- `research::tests::merge_results_respects_limit`
- `research::tests::slugify_produces_filesystem_safe_names`
- `research::tests::build_report_renders_sources_and_links`
- `research::tests::build_report_truncates_excerpt`
- `research::tests::write_report_creates_markdown`（.research 目录落盘 + 回退文件名；rules/03 豁免：PID 级 tempfs、无网络、即用即清，避免为单测公开 `write_report`）
- 既有 chrome_headless normalise_url / not_found_message / format_serp_output 全量保留

### Unit tests（`cargo test -p opencoder-session --features browser --lib`）
- `web_search::tests::wait_selector_maps_all_four_engines`（bing/baidu/sogou/ddg 四引擎 selector）
- `web_search::tests::wait_selector_unknown_engine_falls_back_to_ddg`
- `web_search::tests::engine_dispatch_uses_research_serp_url`（四引擎 URL 复用 `research::serp_url` + 未知引擎回退契约）
- `web_search::tests::parameters_enum_advertises_four_engines`

### Ignored 联网冒烟（`cargo test -p opencoder-session -- --ignored`，需本机 chromium）
- `research::tests::research_smoke_bing_wikipedia` — bing SERP → wikipedia 条目 →
  断言标题非空、正文 > 200 字符（本容器 snap chromium 无真实二进制，未执行）

### 回归
- `cargo test -p opencoder-session --lib`：223 passed / 0 failed / 1 ignored（当次实跑）
- `cargo test -p opencoder-session --features browser --lib`：227 passed / 0 failed / 1 ignored（当次实跑）
- `cargo check -p opencoder-session`（默认 + `--features browser`）：零 error
