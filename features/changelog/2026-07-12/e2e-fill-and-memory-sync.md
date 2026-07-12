Commit: (working-tree)

# e2e 补齐 E12–E14 + agents 本地记忆同步

## 背景

e2e 契约套件（`scripts/e2e/`）覆盖 E1–E11，但 CLI 可达的 `session list`/`session delete`、`config show`、交错思考 reasoning_content 持久化三项缺 e2e 断言。同时 agents 本地记忆滞后于最新代码——`agents/tui` 未记录 app_helpers/selection/测试文件拆分；`agents/llm` 停留在 2026-07-09 未反映 client/sse/tool_call/event 重构；`agents/cli` 缺 e2e 运行手册。

## 变更

### e2e 新增 E12–E14（`scripts/e2e/`）

| ID | 场景 | 契约 | hard/soft |
|----|------|------|-----------|
| E12 | `session list` + `session delete` 生命周期 | 空 workdir 显示无 session → 创建后 list 出现 → delete 成功 → list 中消失 | HARD |
| E13 | 交错思考 reasoning_content 持久化 | 复用 E1 snake session 的 `show --json`，检查 assistant 消息含 `kind: reasoning` block | SOFT |
| E14 | `config show` 输出合法 merged JSON | `opencode config show` 可 `json.loads`，含 `model`/`provider`/`compaction` 字段 | HARD |

- `lib.py` 新增 `session_list`/`session_delete`/`config_show`/`has_reasoning_blocks` 辅助函数。
- `cli_scenarios.py` 加 `import json`、docstring 更新、E12–E14 场景块。
- 语法 gate：`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py` → 零错误。

### agents 记忆同步

- **`agents/tui/index.md`**：`## 关键抽象` 新增 `app_helpers`（app.rs 提取的 pub(crate) 自由函数）、`selection`（鼠标选择 + OSC52 复制）、测试文件拆分说明（`chat_tests.rs`/`render_tests.rs`/`app_tests.rs` 经 `#[path]` include，模块路径不变）。
- **`agents/llm/index.md`**：`## 关键抽象` 全量重写——反映当前 10 子模块结构（client/sse/tool_call/event/message/request/tokens/stream/mock/schema）；新增 `SseDecoder` 不完整 UTF-8 处理、`ToolAccumulator`/`PartialTool`/`CompletedToolCall`、`LlmEvent` 变体清单、`lower_messages` 的 reasoning_content 回传、`MockChatClient` 请求录制接缝。
- **`agents/cli/index.md`**：新增 `## e2e 测试套件` 段——入口/flag/鉴权/观测面/hard-soft 断言模型/语法 gate/E1–E14 场景清单/TUI 专属功能覆盖说明；`## 代表性锚点` 新增 e2e 场景契约引用。

## Impact Surface

- e2e 套件从 E1–E11 扩展到 E1–E14（+3 场景、+4 lib 辅助函数）。
- agents 记忆与代码结构同步：tui 补 app_helpers/selection/测试拆分；llm 全量刷新；cli 补 e2e 运行手册。
- 不影响任何运行时代码——仅 Python 测试 + Markdown 记忆。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [agents/llm](../../agents/llm/index.md)
- [agents/cli](../../agents/cli/index.md)
- [e2e 深度契约 + session show --json](./e2e-deep-contracts-and-session-show-json.md)
