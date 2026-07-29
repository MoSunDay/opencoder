# refactor(tools): 删除 grep/glob/write，统一为 ripgrep 引擎 search 工具 + agent 工具集重构

## 背景
此前 session 工具集存在三个独立但高度重叠的文件检索/写入工具：
- `grep`（正则搜索，依赖 `walkdir` + `regex`，无 `.gitignore` 感知）
- `glob`（文件名匹配，自实现递归遍历，内含 symlink 环路检测）
- `write`（整文件覆写，已被 `edit` 的精确替换覆盖）

三者各有缺陷：`grep` 不尊重 `.gitignore`/`.ignore`，导致搜索结果充斥构建产物与依赖目录噪音；`glob` 的自实现遍历重复造轮子且仅做文件名匹配，与 `grep` 的路径过滤语义重叠；`write` 与 `edit` 功能冲突（前者盲目覆写，后者精确替换），保留两者使模型困惑。

## 设计要点
- **统一 search**：新建 `SearchTool`（`crates/session/src/tools/search.rs`），基于 ripgrep 引擎 crate（`grep-regex` + `grep-searcher`）+ `ignore` walker，**进程内**完成匹配，无需用户安装 `rg` 二进制。天然尊重 `.gitignore`/`.ignore`、跳过 hidden/binary 文件，输出格式 `path:line: content`，最多 1000 条匹配后截断。支持 `pattern`（必填正则）、`path`（可选目录或单文件）、`include`（可选文件名 glob 白名单，如 `"*.rs"`）。
- **删除 grep/glob/write**：移除 `grep.rs`、`glob.rs`、`write.rs` 三个文件及其所有测试；工具不再注册。
- **agent 工具集重构**：`explore` 子 agent 工具由 `[read, glob, grep, ls]` → `[search, read]`；`build` 子 agent 由含 `write`/`read`/`glob`/`grep` 的「全工具」→ `[bash, edit]`；`tools` 伞子 agent 由 `[read, glob, grep, ls]` → `[read, search, ls]`。各 agent 的 prompt 文本同步更新（`base_prompt_explore`/`base_prompt_build`/`base_prompt_tools`）。

## 变更
| 文件 | 改动 |
|---|---|
| `crates/session/src/tools/search.rs` | **新增**（158 行）。ripgrep 引擎 search 工具：`RegexMatcherBuilder` + `Searcher`（line_number）+ `ignore::WalkBuilder`（follow_links, overrides）。`Collector` Sink 收集 `path:line: content`。 |
| `crates/session/src/tools/grep.rs` | **删除**。 |
| `crates/session/src/tools/glob.rs` | **删除**。 |
| `crates/session/src/tools/write.rs` | **删除**。 |
| `crates/session/src/tools/mod.rs` | 注册表移除 grep/glob/write，加入 `SearchTool`；模块声明更新。 |
| `crates/session/Cargo.toml` | 新增依赖 `grep-regex`、`grep-searcher`、`ignore`；移除 `walkdir`（仅 glob 用）。 |
| `crates/core/src/agent.rs` | explore tools `[search, read]`；build tools `[bash, edit]`；tools umbrella tools `[web_fetch, web_search, computer_use, read, search, ls]`；prompt 文本更新。 |
| `crates/core/tests/tool_filter.rs` | `ToolFilter::Allow` 测试用例更新为 search/read/ls。 |
| `crates/session/tests/tools_contract.rs` | 删除 grep/glob/write 测试（9 个），新增 search 测试（8 个）。 |
| `crates/session/tests/capabilities_and_tools.rs` | tools umbrella 断言更新为 read/search/ls。 |
| README.md / README.en.md | 工具列表文档更新。 |

## 测试
新增 search 集成测试（`crates/session/tests/tools_contract.rs`，全部 `#[tokio::test]` + tempdir + ToolContext，零 LLM/网络/DB）：

| 功能 | 测试名 | 断言 |
|---|---|---|
| 正则匹配输出格式 | `search_finds_matching_lines` | 输出含 `relpath:line: content` 精确格式 |
| 无匹配优雅返回 | `search_returns_no_matches_cleanly` | `assert_eq!("no matches")` |
| include 文件名过滤 | `search_include_filter_restricts_files` | `include: "*.rs"` 仅匹配 .rs 文件 |
| 子目录递归 | `search_searches_subdirectories` | 匹配嵌套目录内文件 |
| 非法正则错误 | `search_invalid_regex_errors` | pattern `"*"` 返回 `"invalid regex"` |
| 单文件目标 | `search_single_file_target` | `path: "a.rs"` 仅搜索单文件 |
| 正则锚点 | `search_regex_anchors_work` | `^fn` 锚定行首匹配 1 次 |
| 符号链接跟随 | `search_follows_symlinked_file` | `#[cfg(unix)]`，搜索跟随 symlink 的文件 |

补丁新增 agent 工具集钉定测试（`crates/core/src/agent.rs` `#[cfg(test)]`）：

| 功能 | 测试名 | 断言 |
|---|---|---|
| explore 工具集钉定 | `explore_subagent_carries_search_and_read_only` | allows search+read；deny bash/edit/task/write/glob/grep/ls |
| build 工具集钉定 | `build_subagent_carries_bash_and_edit_only` | allows bash+edit；deny search/read/task/write/glob/grep/ls |

## 验证
| 检查项 | 结果 |
|---|---|
| `cargo build --workspace` | PASS — `Finished dev profile`，零错误 |
| `cargo test -p opencoder-core --lib` | 45 passed; 0 failed（+2 钉定测试） |
| `cargo test -p opencoder-session --lib` | 171 passed; 0 failed |
| `cargo test -p opencoder-session --test tools_contract` | 18 passed; 0 failed（8 search + 4 edit + 1 ls + 5 bash） |
| `cargo clippy -p opencoder-core -p opencoder-session --all-targets -- -D warnings` | PASS — 零警告 |
| 防修绿扫描 | PASS — 删除的 9 个测试均为被删工具（grep/glob/write）自身的测试，非删测试修绿 |

## Impact Surface
- **agent.rs 工具列表**：explore/build/tools 三者的 `ToolFilter::Allow` 已重构。此变更经 `schema_for` + 运行时 `ToolFilter` 自然传播到 LLM 请求 schema 与工具执行。新增钉定测试覆盖（rules/01 结构性变更）。
- **tools/mod.rs 注册表**：grep/glob/write 不再注册，search 新增注册。依赖该注册表的 session runner、schema_for 排序测试均已更新。
- **文件行数**：search.rs 158（≤400），mod.rs 284 / agent.rs 320 / task.rs 84（均≤800）。

## 风险与回退
工具替换边界清晰：explore `[search, read]` / build `[bash, edit]` 语义明确，无功能回归。TUI/cli/web 无对 grep/glob/write 的硬编码依赖（已 grep 确认）。agent 工具列表变更由 `schema_for` + 运行时 `ToolFilter::allows` 自然传播。回退方法：`git checkout` 恢复 grep.rs/glob.rs/write.rs + 还原 mod.rs 注册表 + agent.rs 工具列表。
