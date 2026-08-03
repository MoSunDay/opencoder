Commit: (working-tree, pre-initial-commit)

# fix: resume/replay token 估算对齐流式模型 + 重放 Cancelled 子任务 + 工具专属消息 content 非空

## 背景

本轮修复三个相互独立但同属"消息流水一致性"的缺陷：

1. **resume 后 `context_used` 与流式累积不一致** — resume/replay 用
   `estimate_messages` 计算 `context_used`（含每条消息 +4 结构开销），但
   live 流式追踪（`track_context` / `finalize_assistant`）只计内容、不计
   framing。导致 resume 后状态栏 `ctx%` 比同等历史在流式下累积的值偏高，
   与权威压缩路径（`compaction`）的口径也不一致。子 agent `reconstruct_child_view`
   重建后 `context_used` 恒为 0（事件可能不完整，未回填）。

2. **resume 不重放 Cancelled 子任务** — `resume_and_replay` 只过滤 `Running`
   状态，`Cancelled` 任务永久悬挂、tool_result 永不回填，造成 dangling
   间隙，resume 后首轮对话丢失上下文。

3. **工具专属 assistant 消息 `content` 为 null** — `push_assistant` 对
   "仅含 tool_calls、无文本"的 assistant 消息输出 `content: null`。OpenAI
   spec 允许 null/空串，但部分兼容后端拒绝 null，导致请求失败。

## 变更

### Fix 1：token 估算新增 display 口径（`crates/llm`、`crates/tui`）

- **`crates/llm/src/tokens.rs`**：新增 `estimate_messages_for_display` ——
  逐条累加内容 token，**不含**每条 +4 overhead，与流式追踪模型一致。
  保留原 `estimate_messages`（含 overhead）不变，供压缩触发决策等使用。
- **`crates/llm/src/lib.rs`**：re-export `estimate_messages_for_display`。
- **`crates/tui/src/session_ui/replay.rs`**：
  - `replay_into_chat` 末尾改用 `estimate_messages_for_display`。
  - `replay_messages`（`pub(super)` → `pub`）末尾补 `context_used` 赋值。
  - `reconstruct_child_view` 返回前从 store 加载完整消息列表，用
    `estimate_messages_for_display` 回填 `context_used`（修复事件不全导致的恒 0）。
- **`crates/tui/src/session_ui.rs`**：re-export `replay_messages`。
- 旧调用方（`compaction.rs`、`autopilot/verify.rs` 用 `estimate_messages`）不受影响。

### Fix 2：resume 重放 Cancelled 子任务（`crates/session/src/resume.rs`）

- `resume_and_replay` 过滤条件 `Running` → `Running | Cancelled`，重放所有
  非终态子任务并回填 tool_result（行为变更：旧测试断言"Cancelled 留悬挂"被
  替换为"Cancelled 被重放并完成"）。
- 变量 `running` → `pending`，注释同步更新。
- `resume.rs:175` 的 `plan_input_count: 0` 属上一会话范围外字段（见备注）。

### Fix 3：工具专属 assistant 消息 content null → 空串（`crates/llm/src/message.rs`）

- `push_assistant`：`text.is_empty()` 分支 `Value::Null` → `Value::String(String::new())`。
  OpenAI spec 允许两者，空串被所有兼容后端接受。

## 测试覆盖

| Fix | 测试名 | 文件 | 分层 |
|-----|--------|------|------|
| 1 | `estimate_messages_for_display_excludes_overhead` | `crates/llm/src/tokens.rs` | unit |
| 1 | `child_view_context_used_is_nonzero` | `crates/tui/tests/resume_context_used.rs` | integration |
| 1 | `replay_messages_context_used_is_nonzero` | `crates/tui/tests/resume_context_used.rs` | integration |
| 2 | `resume_and_replay_replays_cancelled_task`（替换旧 `..._leaves_cancelled_tasks_pending_replay`） | `crates/session/tests/resume_cancelled_pending.rs` | integration |
| 2 | `resume_and_replay_mixed_running_and_cancelled` | `crates/session/tests/resume_cancelled_pending.rs` | integration |
| 3 | `assistant_tool_only_content_is_empty_string_not_null` | `crates/llm/tests/lower_messages.rs` | unit |
| 3 | `assistant_text_content_is_preserved` | `crates/llm/tests/lower_messages.rs` | unit |
| 3 | `multi_turn_tool_only_messages_all_have_string_content` | `crates/llm/tests/lower_messages.rs` | unit |

新增 8 个测试、替换 1 个旧行为测试 → **净增 7**。所有新测试确定性（MockChatClient
+ mem_store/tempdir，无网络、无时序断言）。

## 全量回归（当次实跑）

- `cargo test --workspace` → **1672 passed / 0 failed / 1 ignored**
  （ignored = 预存 `research_smoke_bing_wikipedia` e2e，需 Chrome/网络）
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo build --workspace` → 零错误
- 防修绿扫描：无新增 `#[ignore]`、无删除的 `#[test]`、无弱断言、无调试输出
- 行数合规：`resume.rs` 719（迭代 ≤800）；`lower_messages.rs` 432（迭代 ≤800）

## 备注（范围外脏改动）

本工作树另有约 28 个文件来自并发会话的范围外改动（clear-context 计划保留、
TUI compaction 点击展开、queue FIFO 排空、TUI mouse/plan/render、
`plan_input_count` 字段等），各自有独立 changelog（`clear-context-preserve-plan.md`、
`compaction-click-expand.md`、`queue-fifo-drain-all.md`、`plan-mode-subsequent-prompt-tag.md`）。
提交时由 `submit` skill 仅暂存本次 in-scope 文件，排除所有范围外改动。
`resume.rs:175` 的 `plan_input_count: 0` 同属范围外。
