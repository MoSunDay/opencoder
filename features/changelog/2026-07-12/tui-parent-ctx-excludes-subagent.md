Commit: (working-tree, pre-initial-commit)

# TUI 父窗口 ctx% 不再被 subagent 子事件污染

## 背景

用户观察：并发派遣 subagent 后，**在 subagent 返回结论之前**，TUI 状态栏的父窗口 ctx% 就持续暴涨。直觉上 subagent 应与父隔离，结论未出时父 context 不该动。

### 诊断

- 根因在 `crates/tui/src/chat.rs::ChatView::track_context`：该函数对
  `SessionEvent::SubagentChild { ev, .. }` **递归调用自身**，把子 agent 的
  TextDelta / ToolStart / ToolEnd 等 token 累加进**父** ChatView 的 `context_used`。
- `run_subagent`（`crates/session/src/runner.rs:564-570`）把子的全部事件包装成
  `SubagentChild` 转发给父的 `on_event` —— 子每产生一段文本/工具调用，token 就实时
  加到父的状态栏数字上。故「结论未出，父 ctx% 已暴涨」属实。
- 同时父 `apply`（`chat.rs:175-184`）已把 `SubagentChild` 解包路由给子 ChatView 的
  `view.apply(ev)`，子自己 `track_context` 维护自身计数。因此父的递归是**重复计算 +
  污染父**，而非必需。

### 关键澄清：隔离并未失效

父 agent 真实发给 LLM 的 context **没有**暴涨：
- 父 LLM 请求只用 `session.messages`（`runner.rs:321`），子用独立 `SessionState`
  （`runner.rs:465`，`child_session_id = "sub-<ulid>"`），子消息从不进 `parent.messages`。
- subagent 运行期间父不发 LLM 请求（阻塞在 `execute_call`，`runner.rs:384`）。
- subagent 完成后，只把**结论文本**作为 `ToolResult` 加进父（`runner.rs:288-293, 603`）。

暴涨的只是 **TUI 状态栏的 ctx% 估算值**。

### 这是对 2026-07-11 决策的 reversal

`features/changelog/2026-07-11/subagent-view-and-ctrl-d-fix.md:33` 当时**故意引入**
了 `SubagentChild` 递归，目标是「父 view 的 context_used 含全部后代 token」。实际效果
与用户对「subagent 隔离」的预期冲突，且与 `ChatView.context_used` 字段文档
（`chat.rs:58-60`「excludes child subagent tokens」）自相矛盾。本次按用户意图
（「ctx% 反映当前聚焦窗口的 agent」）反转该决策。

## 变更

### 移除父级 SubagentChild 递归（核心）
- **`crates/tui/src/chat.rs::track_context`**：删除
  `SessionEvent::SubagentChild { ev, .. } => { self.track_context(ev); }` 分支。
  删后该事件落到现有 `_ => {}` 兜底，父 `context_used` 不再含子 token。
- **文档订正**：`track_context` 函数文档从「including child subagent tokens ...
  parent includes all descendants」改为「OWN transcript only ... child ChatView
  tracks its own subtree」，与字段文档（`chat.rs:58-60`）一致。

### 测试强化
- **`crates/tui/src/chat.rs::subagent_events_render`**：
  - 先 apply 一段父自身 `TextDelta`（让父 `context_used` 非零），消除原断言
    「等价于 == 0」的脆弱性。
  - 新增 precondition `assert!(parent_ctx > 0)`。
  - 断言 apply `SubagentChild` 后父 `context_used` **不变**（`assert_eq!`），
    固化「父不得含子 token」语义防回归。
  - 完成态边界：apply `SubagentEnd` 后断言父 `context_used == parent_ctx + summary token`，
    即父只获得 summary，不获得子过程 token。

### 端到端实证（替代手动 TUI 验证）
- **`crates/tui/tests/subagent_context_isolation.rs`**（新增）：用 `MockChatClient` 驱动真实
  `session::run`，收集完整 `SessionEvent` 流（含 `SubagentChild`），replay 进 `ChatView`。两个测试：
  - `real_subagent_stream_does_not_inflate_parent_context`：单个 subagent。
  - `concurrent_subagents_do_not_inflate_parent_context`：**一轮派两个并发 subagent**
    （`two_task_turn`），直接覆盖用户原始「并发 subagent」诉求。
- **断言设计要点**：逐 `SubagentChild` 验证 apply 前后父 `context_used` 不变（非窗口 before/after）。
  并发时 `SubagentEnd`（其 summary 合法计入父）会交织在两组 `SubagentChild` 之间，窗口跨度会混淆。
  逐事件断言精确隔离「子事件不计父」契约。调试轨迹证实：每个 `SubagentChild` apply 前后父值恒定
  （22→22、31→31），`SubagentEnd` 的 +9 是 summary，属合法。
- `crates/tui/Cargo.toml` 新增 `[dev-dependencies] tempfile = "3"`。
- **e2e Python 语法 gate**：`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py` → OK
  （e2e 重构为 Python 契约套件，确认可导入；不在 cargo test 范围）。

### 显示链路（未改，本就正确）
`app.rs:194-209` 已按焦点 view 取 `context_used`：聚焦父窗口取
`chat.context_used`，聚焦子窗口取 `view.context_used`。本次修复使父值不再被污染，
显示链路无需改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 父 context 不含子 token（precondition 非零 + apply SubagentChild 后父值不变） | `subagent_events_render` | `crates/tui/src/chat.rs`（强化） |
| 完成态边界：SubagentEnd 后父只 +summary token | `subagent_events_render` 末断言 | `crates/tui/src/chat.rs`（强化） |
| **端到端**：真实 session::run subagent 事件流 replay 进 ChatView，父 context_used 在子事件期间不增长 | `real_subagent_stream_does_not_inflate_parent_context` | `crates/tui/tests/subagent_context_isolation.rs`（**新增**） |
| **并发端到端**：一轮两并发 subagent，逐 SubagentChild 断言父值不变（覆盖用户原始「并发」诉求） | `concurrent_subagents_do_not_inflate_parent_context` | `crates/tui/tests/subagent_context_isolation.rs`（**新增**） |
| 子 view 独立 track 自身 context（既有，未回归） | `subagent_events_render` 内 `view.context_used > 0` | `crates/tui/src/chat.rs` |
| subagent 隔离语义（父 LLM 请求只用 session.messages） | 既有 `subagent.rs` 6 项 | `crates/session/tests/subagent.rs`（未改，全绿） |

- 全量回归：`cargo test --workspace` → **299 passed / 0 failed**
  （tui: 110 unit + 2 端到端集成 = 112；余为 cli/core/llm/session/store/web）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- e2e：`python3 -m py_compile scripts/e2e/*.py` → OK
- 行数：`chat.rs` 800 ≤ 800；`subagent_context_isolation.rs` 203 ≤ 400

## Impact Surface

- **TUI 用户**：subagent 运行不再让父窗口 ctx% 虚高。ctx% 现精确反映「当前聚焦窗口
  的 agent」—— 父窗口只显父自身，切到 subagent 视图才显该 subagent。
- **无功能隔离变化**：父 agent 真实 LLM context 行为未变（隔离本就有效）。
- **不影响** CLI / Web / session / store 层 —— 改动仅在 `crates/tui/src/chat.rs`。

## Related Docs
- [2026-07-11 subagent 视角（本次反转其 track_context 递归决策）](../2026-07-11/subagent-view-and-ctrl-d-fix.md)
- [agents/tui](../../agents/tui/index.md)（已同步修正）
