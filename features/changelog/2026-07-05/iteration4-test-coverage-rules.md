Commit: (working-tree, pre-initial-commit)

# 迭代四：测试覆盖补齐 + 仓库规则落地

## Context
迭代三结束时 workspace 有 58 个测试，但审计发现多个业务功能零覆盖：LLM 流式原语（SseDecoder/ToolAccumulator）、CLI 全部命令、6/9 工具、subagent 分发、TUI handle_key/ChatView、Web HTTP 层、prompt 构建。本迭代系统性补齐，并建立仓库级测试规则确保未来不再退化。

## Change Summary

### 仓库规则（rules/）
- 新增 `rules/` 目录，含 4 个规则文件：
  - `README.md` — 规则索引 + PR 快速检查清单
  - `01-mandatory-tests.md` — 每个业务功能必须有对应测试；禁止表面测试；达标标准 + 违规处理
  - `02-regression-gate.md` — 迭代收尾全量回归 + changelog 附「功能→测试名」映射
  - `03-test-pyramid.md` — 测试分层（unit 内联 / integration tests/ / e2e scripts/）+ 放置决策树
- `agents.md` 追加「仓库规则」小节引用 `rules/`

### 记忆文档修复
- `agents.md` / `features/index.md`：删除不存在的 `--resume` flag（实际是 `--session <id>`）
- `docs/perf.md`：测试数 46 → 58 → 144
- `iteration3-tui-overhaul.md`：SubagentStart/End 补 `id` 字段；颜色阈值修正（60–84% 黄）；补 `T` 后缀；补 `ReasoningDelta` 累加
- `agents/session/index.md`：SessionState 补 `store` 字段；补 subagent/task + doom-loop 记述

### 测试补齐（+86 个新测试）

| Crate | 新增测试 | 文件 | 覆盖功能 |
|-------|---------|------|---------|
| llm | +7 | `sse.rs`/`tool_call.rs`/`tokens.rs` 内联 | SseDecoder 分帧/[DONE]/CRLF/partial flush/parse_chunk；ToolAccumulator apply/finish_all/fallback；estimate_transcript |
| cli | +13 | `tests/cli_parse.rs` + `run.rs` 内联 | clap 解析（Run/Tui/Serve/Config/Models/Session）；summarize_input/truncate/indent_first |
| session | +17 | `tests/tools_contract.rs`/`subagent.rs`/`prompt.rs` | Write/Edit/Glob/Ls/PlanExit 工具；subagent 事件转发；build_system/environment_block/compaction_prompt |
| tui | +21 | `app_tests.rs`/`chat.rs` 内联 | handle_key（Enter/Ctrl+O/Ctrl+J/Ctrl+T/Left/Right/Ctrl+C）；ChatView::apply（TextDelta/SubagentStart/SubagentEnd/Error） |
| web | +3 | `tests/web_contract.rs` 追加 | list_sessions/get_session/post_model HTTP 层 |
| store | +2 | `tests/store_integration.rs` 追加 | last_message_seq；Delivery::parse/as_str |
| core | +5 | `tests/tool_filter.rs` + `skill.rs` 内联 | ToolFilter::allows（All+Allow list）；truncate_output；ToolOutput ok/err；skill discover |
| **合计** | **+86** | | |

### 基础设施修复
- 创建 `crates/core/src/skill.rs`（lib.rs 声明了 `pub mod skill` 但文件缺失，阻塞编译）
- app.rs 测试拆分至 `app_tests.rs`（app.rs 853→733 行，恢复 ≤800 限制）

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| SseDecoder 分帧 | `drain_splits_on_double_newline` | `llm/src/sse.rs` |
| SseDecoder [DONE] 跳过 | `drain_skips_done_marker` | `llm/src/sse.rs` |
| ToolAccumulator Start+Delta | `apply_emits_start_on_first_seen_then_delta` | `llm/src/tool_call.rs` |
| estimate_transcript 组合 | `estimate_transcript_combines_system_and_messages` | `llm/src/tokens.rs` |
| CLI clap 默认解析 | `default_is_run_with_no_prompt` | `cli/tests/cli_parse.rs` |
| CLI --session/--continue/--fork | `session_flag_sets_id`/`continue_and_fork_flags` | `cli/tests/cli_parse.rs` |
| summarize_input | `summarize_input_extracts_command` | `cli/src/run.rs` |
| WriteTool | `write_tool_creates_file_with_content` | `session/tests/tools_contract.rs` |
| EditTool 替换/错误/replace_all | `edit_tool_replaces_exact_string` 等 | `session/tests/tools_contract.rs` |
| GlobTool 匹配 | `glob_tool_matches_pattern` | `session/tests/tools_contract.rs` |
| PlanExit | `plan_exit_writes_plan_file` | `session/tests/tools_contract.rs` |
| subagent 事件 | `subagent_emits_start_and_end_events` | `session/tests/subagent.rs` |
| subagent 工具转发 | `subagent_forwards_child_tool_events_to_parent` | `session/tests/subagent.rs` |
| build_system | `build_system_includes_agent_prompt_and_environment` | `session/tests/prompt.rs` |
| handle_key Enter | `enter_submits_non_empty_input` | `tui/src/app_tests.rs` |
| handle_key Ctrl+O steer | `ctrl_o_while_running_admits_steer` | `tui/src/app_tests.rs` |
| ChatView::apply SubagentStart | `apply_subagent_start_adds_subagent_header` | `tui/src/chat.rs` |
| Web list_sessions | `list_sessions_returns_created_sessions` | `web/tests/web_contract.rs` |
| Web post_model | `post_model_switches_stored_meta` | `web/tests/web_contract.rs` |
| Store last_message_seq | `last_message_seq_tracks_appends` | `store/tests/store_integration.rs` |
| Delivery parse | `delivery_parse_and_as_str_roundtrip` | `store/tests/store_integration.rs` |
| ToolFilter allows | `tool_filter_allow_list_gates_tools` | `core/tests/tool_filter.rs` |
| truncate_output | `truncate_output_long_content_gets_preview` | `core/tests/tool_filter.rs` |

- 全量回归：`cargo test --workspace` → **144 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## Notes / Compatibility
- `handle_key` 和 `KeyAction` 改为 `pub(crate)` 以支持测试拆分文件
- 测试拆分使用 `#[path = "app_tests.rs"]` 模式，保持 app.rs ≤800 行
- skill.rs 为新建模块（lib.rs 已声明但文件缺失），含 discover/discover_in/skills_dir/Skill + 2 个单测
