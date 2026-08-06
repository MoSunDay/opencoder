Commit: (working-tree, pre-initial-commit)

# chrome-headless 技能增强：代理穿透 + 站内搜索引擎矩阵

## 背景
chrome-headless 技能此前有两个短板：
1. **无代理穿透**——`dump_dom` 与 4 个 `do_*` 工具函数硬编码不传 proxy，无法在受限/需要
   出口代理的网络环境中使用浏览器抓取。
2. **SERP 解析与站点搜索零散**——搜索引擎结果页（SERP）解析逻辑混在 `web_read.rs`，仅
   覆盖 bing/baidu/sogou/ddg；缺少 google 与 GitHub/HuggingFace 站内搜索能力，研究流程
   对站内资源（代码仓库、模型卡片）的检索覆盖不足。

本批次分 7 个阶段一次性完成代理支持、SERP 模块拆分、新增 google/github/hf 三引擎、
SKILL 文档与测试补全。

## 变更

### Phase 1 — Chrome 代理穿透
- **`crates/session/src/tools/chrome_headless.rs`**：
  - `chrome_base_args(ua, proxy)` 与 `dump_dom(url, wait_ms, ua, proxy)` 现接受可选 proxy。
    proxy 非空时注入 `--proxy-server`，并同时注入 `--proxy-bypass-list`。
  - 新增常量 `CHROME_PROXY_BYPASS = "<local>;127.0.0.1;localhost;::1;0.0.0.0"`
    （注意 Chrome bypass 用 `;` 分隔，而非 `,`）。
  - 4 个 `do_*`（fetch/resolve/html/screenshot）统一 `effective_proxy(ctx.proxy.as_deref())`
    取值后透传，空串/纯空白视为无代理。
- **`crates/session/src/tools/research.rs`**：两处 `dump_dom` 调用均透传 `proxy.as_deref()`。

### Phase 2 — SERP 模块拆分
原 `web_read.rs` 承载过多职责，拆为高内聚的 SERP 解析层与纯网页阅读层：
- **`crates/session/src/tools/serp.rs`**（新增，108 行）：`SearchResult` 结构体、
  `parse_search_results` 主调度器（按 URL host 分派）、共享 helper（`normalize_ws` 等）、
  `#[path = "serp_engines.rs"] mod engines;`。
- **`crates/session/src/tools/serp_engines.rs`**（新增，378 行）：7 个引擎解析器
  （bing/baidu/sogou/ddg/google/github/hf）。
- **`crates/session/src/tools/serp_tests.rs`**（新增，361 行）：全部 SERP 单元测试。
- **`crates/session/src/tools/web_read.rs`**：瘦身至 235 行，仅保留网页阅读。
- **`crates/session/src/tools/mod.rs`**：注册 `pub mod serp;`。

### Phase 3-5 — Google / GitHub / HuggingFace 站内搜索
- **`research::serp_url`** 扩展为支持 7 引擎：`baidu`/`sogou`/`ddg`/`google`/
  `github`/`hf`/`huggingface`（`bing` 为未知引擎的默认兜底）。`web_search` 工具的
  `items` enum 同步更新。
- **`serp_engines.rs`**：
  - `parse_google_results`：`div.g` 卡片 + `h3` 标题 + `/url?q=` 反解。
  - `parse_github_results`：`/{owner}/{repo}` 抽取并去重。
  - `parse_hf_results`：模型卡片链接抽取。
- **`crates/session/src/tools/web_extract.rs`**：`SITE_PROFILES` 新增 HuggingFace 条目
  （`huggingface.co`），用 og:title 与模型卡片做结构化抽取。

### Phase 6 — SKILL.md 文档
- **`crates/core/assets/skills/chrome-headless/SKILL.md`** 更新至 114 行：新增 proxy 章节、
  站内搜索引擎矩阵表、HF 进入抽取清单、工作流示例。

### Phase 7 — 测试
- `chrome_headless.rs` 新增 4 个代理单测。
- `web_extract_tests.rs` 新增 HF 抽取测试。
- `serp_tests.rs` 新增 google/github/hf 解析器测试 + host 分派测试。
- `research_tests.rs` 的 `serp_url` 测试覆盖全部 7 引擎。
- `web_search.rs` browser-gated 测试修正：fallback 用例由 `"google"` 改为
  `"unknownengine"`（google 已是受支持引擎，不再适合做兜底断言）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| proxy 时注入 --proxy-server | chrome_base_args_includes_proxy_server_when_provided | crates/session/src/tools/chrome_headless.rs |
| 无 proxy 时不注入 | chrome_base_args_omits_proxy_when_none | crates/session/src/tools/chrome_headless.rs |
| proxy 注入 bypass-list | chrome_base_args_includes_proxy_bypass_list | crates/session/src/tools/chrome_headless.rs |
| 空白 proxy 视为无代理 | chrome_base_args_ignores_empty_proxy | crates/session/src/tools/chrome_headless.rs |
| SERP host 分派 | parse_search_results_dispatches_by_host / parse_search_results_dispatches_all_engines | crates/session/src/tools/serp_tests.rs |
| Google 解析（标题/url/snippet/limit/空） | parse_google_extracts_title_url_snippet / parse_google_respects_limit / parse_google_handles_empty | crates/session/src/tools/serp_tests.rs |
| GitHub 仓库解析（抽取/去重/空） | parse_github_extracts_repos / parse_github_dedups / parse_github_handles_empty | crates/session/src/tools/serp_tests.rs |
| HF 模型解析（抽取/空） | parse_hf_extracts_models / parse_hf_handles_empty | crates/session/src/tools/serp_tests.rs |
| serp_url 7 引擎 + 兜底 | serp_url_builds_per_engine_urls | crates/session/src/tools/research_tests.rs |
| HF 结构化抽取 | huggingface_uses_og_title_and_model_card | crates/session/src/tools/web_extract_tests.rs |
| web_search 兜底用例修正 | (browser-gated) serp_url("unknownengine", …) | crates/session/src/tools/web_search.rs |

- `cargo build --workspace` ✓
- `cargo test -p opencoder-session --lib` → 305 passed, 0 failed ✓
- `cargo clippy -p opencoder-session --lib -- -D warnings` ✓
- `cargo test --workspace` ✓
- 行数：serp.rs 108 ≤ 400；serp_engines.rs 378 ≤ 400；serp_tests.rs 361 ≤ 400；
  web_read.rs 235 ≤ 800；chrome_headless.rs 665 ≤ 800；web_extract.rs 342 ≤ 800

## Impact Surface
- chrome-headless 工具：新增可选代理穿透，既有无代理路径行为不变。
- web_search / research：引擎矩阵从 4 扩至 7，新增 google/github/hf 站内搜索与解析；
  `serp_url`/`parse_search_results` 为纯函数，feature 独立，默认构建即可单测。
- web_extract：HF 站点进入结构化抽取清单，其它 SITE_PROFILES 不变。
- 不影响：session runner / store / web / cli 边界。

## Related Docs
- [agents/session](../../agents/session/index.md)
