Commit: (working-tree, pre-initial-commit)

# feat(session): multi-engine SERP parsing (Baidu/Bing/Sogou) + structured search output

## 背景
`web_read` 此前仅能解析 DuckDuckGo（DDG）HTML 结果页（`parse_ddg_results`）。
当用户或上游经 `chrome_headless` 的 fetch 访问百度 / Bing / 搜狗等中文搜索结果页时，
返回的是一整页 HTML，只能走通用的 `extract_readable_text` 抽取正文，结果夹杂大量
导航 / 侧栏噪声、分页链接与广告，对模型几乎不可用。

本次为 `web_read` 补齐百度 / Bing / 搜狗三大中文搜索引擎的纯函数 SERP 解析器，
并新增按 URL 域名分发的 `parse_search_results` 总入口；`chrome_headless` 的
`do_fetch` 据此自动识别 SERP 页面并输出结构化（标题 / URL / 摘要）结果，非搜索页
保持原有可读正文抽取行为不变。

## 变更
### 解析器与分发器（session/tools/web_read.rs）
- 新增三个纯函数解析器（零 I/O，可默认 build 单测）：
  - `parse_baidu_results`：解析 `div.c-container`（organic `result` 与 one-box
    `result-op`），标题取首个 `h3 a`，摘要取 `.c-abstract`（缺失则回退容器自身
    文本并剥去开头的标题块）；结果链接是百度跳转（`/link?url=` / `baidu.php`），
    真实目标无法客户端解码，故保留跳转 href 原样（仅 `&amp;`→`&`）。
  - `parse_bing_results`：解析 `li.b_algo`，标题取 `h2 a`，href 为**直接目标 URL**
    （无需跳转解码），摘要取 `p.b_lineclamp*` / `.b_caption p`（再回退容器文本
    去标题块）。
  - `parse_sogou_results`：解析 `div.vrwrap` / `div.rb`，标题取 `h3 a`，链接为相对
    跳转 `/link?url=`，按 `https://www.sogou.com` 补全为绝对；摘要取
    `.str_info` / `.str-text-info` / `.fz-mid` / `.space-txt`（再回退）。标题中的
    `<em>` 与 `<!--red_beg-->` 注释节点由 scraper 的 `text()` 自动跳过，无需额外清洗。
- 新增 `parse_search_results(url, html, limit)`：按 URL 域名分发到对应引擎解析器
  （baidu→baidu、bing→bing、sogou→sogou、duckduckgo→ddg），非搜索域名返回空 Vec
  （交由调用方回退可读正文抽取）。
- 新增共享清洗纯函数：`normalize_ws`（`\u{a0}`→空格并折叠连续空白，防止百度
  `&nbsp;` 泄漏为杂散字符）、`normalize_baidu_href`、`normalize_redirect_href`
  （相对 href 补全 + `&amp;` 转义）、`first_snippet`（按优先级取首个匹配片段）、
  `container_text_minus_title`（容器文本去开头标题块）。
- 所有解析器对标题去重（`HashSet`）丢弃重复广告行，空标题 / 空 href 跳过，`limit`
  截断。

### 结构化输出（session/tools/chrome_headless.rs）
- `do_fetch`：成功取回 HTML 后，先尝试 `parse_search_results(url, html, 12)`；
  非空则经新增 `format_serp_output` 渲染为带源 URL 标题的编号 markdown 列表
  （`# Search results: <url>` + 每行 `**标题**` / 摘要 / URL），空字段省略；为空则
  保持原 `extract_readable_text` + `# {url}\n\n{text}` 行为不变。新增 `url` crate
  依赖以解析域名做分发判定。

> 路径一致性修复（`~/.opencode`→`~/.opencoder`）由独立 changelog
> `2026-07-23/tui-ctrlL-hint-and-opencoder-path-consistency.md` 覆盖，此处不重复。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 百度：标题 / URL / 摘要提取 | `parse_baidu_extracts_title_url_snippet` | web_read.rs |
| 百度：limit 截断 | `parse_baidu_respects_limit` | web_read.rs |
| Bing：标题 / URL / 摘要提取 | `parse_bing_extracts_title_url_snippet` | web_read.rs |
| Bing：limit 截断 | `parse_bing_respects_limit` | web_read.rs |
| 搜狗：标题 / URL / 摘要（含相对跳转补全、`&amp;`→`&`、nbsp 不泄漏、导航噪声过滤） | `parse_sogou_extracts_title_url_snippet` | web_read.rs |
| host 分发器：baidu / bing / sogou / ddg 命中、非搜索域回退空 | `parse_search_results_dispatches_by_host` | web_read.rs |
| do_fetch 结构化输出渲染（编号、空字段省略） | `format_serp_output_renders_markdown_list` | chrome_headless.rs |

- 全量回归：`cargo test --workspace` → **869 passed; 0 failed; 0 ignored**（隔离 target dir 复跑）
- clippy（SERP 子集 `web_read.rs` / `chrome_headless.rs`）：零命中
- 行数：`web_read.rs` 777（< 800，迭代中）、`chrome_headless.rs` 345（< 400）
- 分层（rules/03）：全部为纯函数单元测试（内联 `#[cfg(test)]`），零网络 / 时序 / 并发，< 10ms

## Impact Surface
- 用户：经 `chrome_headless` fetch 中文搜索结果页时，输出从噪声正文变为干净的结构化
  结果（标题 / URL / 摘要），显著提升模型对中文 SERP 的可用性。
- 兼容性：`do_fetch` 的 SERP 分支为**可选增强**——非搜索页行为完全不变；新增解析器
  均为独立纯函数，不影响既有 DDG / Baidu 逻辑路径（`parse_search_results` 仅在域名
  命中时调用对应引擎，DDG 既有分支未改动）。
- 依赖：`chrome_headless` 新增 `url` crate（已在 workspace，无新外部依赖）。

## Related Docs
- 触及区逻辑文档：`agents/session/index.md`（web_read SERP 能力描述，repair-on-touch 同步）
- 相关能力：`web_fetch` / `web_search`（feature-gated `browser`，经 obscura）
