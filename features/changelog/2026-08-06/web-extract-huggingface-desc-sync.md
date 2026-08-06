Commit: (working-tree, pre-initial-commit)

# web_extract 描述串 + session memory 同步 huggingface profile

## 背景
上一轮提交（`abd3e76`）为 `web_extract` 落地了 HuggingFace 提炼 profile（`huggingface.co` host，og:title 取标题、model card 正文取正文）并把 SERP 解析拆成 `serp.rs`(+`serp_engines.rs`)。但两处**表象层未跟上**：
- `WebExtractTool::description()` 的站点列表仍写 "36kr, stackoverflow"，模型看不到 huggingface 支持。
- `agents/session/index.md` 的能力门控工具集条目仍是旧版单体 `serp.rs` 表述，未反映 `serp_engines.rs` 拆分与 huggingface profile。

本变更是 repair-on-touch 的记忆/描述同步，使模型可见描述与底层实现、memory 语义模型三方一致。

## 变更

### 工具描述串补全 huggingface
- **`crates/session/src/tools/web_extract.rs:308-309`**：`description()` 站点列表由 "36kr, stackoverflow" 改为 "36kr, stackoverflow, huggingface"，与已实现的 profile（`:117-126`，host `huggingface.co`）对齐。模型据此能正确路由 HF 页面到 `web_extract`。

### session memory 同步模块结构
- **`agents/session/index.md`**：重写「能力门控工具集 + capability filter」条目——① 标注 `serp` 现为 `serp.rs`(+`serp_engines.rs`) 双文件拆分（`parse_search_results` 按 host 分派 + 各引擎 CSS 选择器解析器）；② `web_extract` 标注「10 个站点固定分析 profile（新增 huggingface）」；③ 精简 `research`/`chrome_headless` 表述（proxy 注入语义、`effective_proxy` 取值）。无代码语义变更。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| huggingface profile 提炼 | (既有 web_extract profile 测试覆盖) | crates/session/src/tools/web_extract.rs |
| hf 模型搜索解析 | parse_hf_results (既有单测) | crates/session/src/tools/serp_engines.rs |

- 全量回归：`cargo test --workspace` → 1959 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净
- 行数：web_extract.rs（迭代中文件）< 800；index.md 为 memory 文档

## Impact Surface
- 模型可从 `web_extract` 工具描述串中看到 huggingface 支持，更准确地把 HuggingFace 页面路由到结构化提炼。
- **不影响**：工具执行逻辑、提取算法、Store / LLM / session runner 边界——纯描述串 + 文档同步。

## Related Docs
- [agents/session](../../agents/session/index.md)
- 上一轮：[chrome-headless proxy passthrough + SERP engine matrix](../2026-08-06/../../commit abd3e76)（commit `abd3e76`）
