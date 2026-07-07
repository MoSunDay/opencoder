# 子 agent 支持 + 工具重构 + TUI 交互优化

## 变更摘要

### 子 agent 系统（tokio 任务 + libsql 父子追踪）
- **新增 subagent_tasks 表**（schema.rs）：记录父-子 agent 关系，包括提交提示词、最终结果、状态（running/completed/failed）、开始/完成时间。
- **Store API 扩展**：`create_subagent_task`、`complete_subagent_task`、`list_subagent_tasks` 三个新方法加入 Store trait + libsql 实现。
- **run_subagent 重写**（runner.rs）：子 agent 使用独立 SessionState（含独立 session_id `sub-<ulid>`）和 store 连接。运行前记录 parent-child 关系，运行后将子 agent 的最终输出写入 `subagent_tasks.result`。子 agent 事件持久化到 `session_events` 表，可供回放/JSONL 导出。
- **SessionEvent: Serialize**：所有事件类型可序列化，支持 JSONL 存储格式。

### 工具重构：主 agent 只保留 bash + task
- **act agent**：`Allow(["bash", "task"])` — 编排型 agent，通过 bash 执行终端操作，通过 task 委派文件操作给子 agent。
- **plan agent**：`Allow(["bash", "task", "plan_exit"])` — 只读规划，bash 写命令被拦截。
- **新增 explore 子 agent**：只读工具（read/glob/grep/ls/bash），用于代码搜索调研。
- **新增 build 子 agent**：完整文件工具（read/write/edit/bash/glob/grep/ls），用于代码实现修改。
- **command agent**：`Allow(["bash", "task"])`，与 act 一致。
- **提示词更新**：BASE_PROMPT 指导主 agent 将文件操作委托给 explore/build 子 agent；PLAN_SUFFIX 明确说明 bash 写命令会被拦截。
- **task 工具描述更新**：引导模型选择 explore（只读）或 build（可写）子 agent。

### bash 写命令拦截器（plan 模式）
- **新增 bash_guard 模块**（bash_guard.rs, 319 行）：纯函数式 `classify(command) -> BashVerdict`，检测重定向（>/>>/&>）、变异命令（rm/mv/cp/mkdir/dd 等）、Git 写操作（push/commit/merge/reset/checkout --）、包管理器（apt/pip/npm/cargo install）、就地编辑（sed -i/perl -i/awk -i）、间接执行（exec/eval/source）。
- **runner 集成**：plan 模式下 `execute_call` 在 bash 执行前调用 `bash_guard::classify`，若为 WriteBlocked 则返回描述性错误给模型（"Blocked in plan mode: ..."），让模型理解当前约束。
- **11 个单元测试**：覆盖只读命令、重定向、变异命令、Git 读/写、包管理器读/写、就地编辑、复合命令、sudo 前缀。

### 移除 max_steps
- **Agent struct**：移除 `max_steps` 字段。
- **Config struct**：移除 `max_steps` 字段 + 配置解析。
- **runner.rs**：移除 step 计数和 `max_steps` 检查。agent 终止条件变为：模型不再调 tool（Done）/ 用户双击 Esc / doom-loop 检测到 3 次重复调用。
- **SessionState**：移除 `step` 字段。
- 活没干完就继续干，不应被人为步数上限截断。

### TUI 修复

#### Thinking 默认折叠
- `ensure_thinking_open()` 创建的 Thinking 块默认 `collapsed: true`。折叠时显示 `💭 Thinking (N lines) [↓ expand]`。

#### say: 助手标识
- Assistant 块渲染时增加 `say:` 绿色粗体头标行（对齐 user: 标识），后续行 4 空格缩进。流式和 Markdown 渲染后均有此标识。

#### 快捷键重构
- **Enter**：空闲=提交 prompt，运行中=Steer（强干预，turn boundary 提交，reset step budget）
- **Tab**：空闲=提交 prompt，运行中=Follow-up 队列（弱排队，任务完成后提交）
- **Alt+Tab**：切换 act↔plan 模式（Ctrl+T 保留为不支持 Alt+Tab 的终端的 fallback）
- **Shift+Enter**：插入换行（多行输入），Alt+Enter 不再触发换行
- 移除 Ctrl+O（Steer）、Ctrl+J（Queue）— 由 Enter/Tab 取代

#### Skill 模糊匹配
- `fuzzy_score(query, target)` 子序列匹配算法：奖励紧凑连续匹配、早期匹配。空查询显示全部。
- `build_rows` 改用 fuzzy_score 过滤+排序，替代原 substring 匹配。

### 每 session 独立状态
- **新增 session_ui.rs**：`SessionUiState` 快照结构，捕获 chat/history/scroll/context/steer_count/queue_count/active_skill/agent_name。
- **`/task` 切换保存/恢复**：切换前将当前 session 的完整 UI 状态存入 `HashMap<session_id, SessionUiState>`，切回时恢复。包括 chat 历史和滚动位置。worker 在切换时被终止（running=false），切回后需重新提交。

## 涉及文件

| 文件 | 变更 |
|------|------|
| `crates/core/src/agent.rs` | 移除 max_steps，重构 agent 工具集，新增 explore/build agent |
| `crates/core/src/config.rs` | 移除 max_steps 字段 |
| `crates/session/src/bash_guard.rs` | **新增** — bash 写命令分类器（319 行，11 测试） |
| `crates/session/src/runner.rs` | run_subagent 重写 + libsql 追踪 + plan 模式 bash 拦截 |
| `crates/session/src/lib.rs` | 注册 bash_guard 模块 |
| `crates/session/src/tools/task.rs` | 工具描述更新 |
| `crates/store/src/libsql_store/schema.rs` | subagent_tasks 表 + 索引 |
| `crates/store/src/libsql_store/subagent_tasks.rs` | **新增** — libsql CRUD |
| `crates/store/src/store.rs` | Store trait 新增 3 个方法 |
| `crates/store/src/types.rs` | SubagentStatus + SubagentTaskRecord |
| `crates/tui/src/chat.rs` | thinking 折叠 + say: 标识 + Clone derive |
| `crates/tui/src/app.rs` | 快捷键重构 + session 状态保存/恢复 |
| `crates/tui/src/keybind.rs` | 帮助文本更新 |
| `crates/tui/src/menu.rs` | fuzzy_score 模糊匹配 |
| `crates/tui/src/session_ui.rs` | **新增** — 每 session UI 状态快照 |
| `crates/tui/src/app_tests.rs` | 快捷键测试更新 |
| `crates/core/tests/config_contract.rs` | 移除 max_steps 断言 |
| `crates/session/tests/subagent.rs` | subagent → explore 适配 |
| `crates/session/tests/steer_followup.rs` | 移除 max_steps |

## 测试
- 全量 **201 passed**（初次 186 → 审查修复后 201），`clippy --all-targets -- -D warnings` 全绿。
- Release 二进制 10 MB，已安装至 `/usr/local/bin/opencoder` + `/root/.cargo/bin/opencoder`。

## 审查修复（go-live gate 补齐）
上线前审查发现并修复的问题：
- **bash_guard 集成测试**（P0）：新增 `tests/bash_guard_plan_mode.rs`（3 测试）——plan 模式拦截 `rm -rf` / 放行 `ls` / act 模式不受限。此前只有单元测试，无 runner 级验证。
- **subagent libsql 持久化测试**（P0）：`subagent.rs` 新增 2 测试验证父子关系 + 子事件持久化到 store；`store_integration.rs` 新增 3 测试覆盖 CRUD 往返 / 按 parent 过滤 / status 序列化。此前测试未挂 store，持久化路径被 `if let Some(store)` 静默跳过。
- **bug 修复**：`run_subagent` 未为子 agent 创建 `sessions` 行导致 FK 约束失败，subagent_tasks 记录静默丢失。已添加 `store.create_session()` 调用。
- **`parent_message_id`**：此前始终 `None`（死数据）。现从 parent 最后一条 assistant 消息取 ID 填充。
- **`subagent_type` 校验**：此前非法值（如 `"foobar"`）静默降级为 explore。现返回描述性错误（"Unknown subagent_type '...'. Valid options: 'explore' or 'build'."），新增 `subagent_rejects_unknown_type` 测试。
- **session_ui 可测试性**：提取 `snapshot()` 纯函数 + 4 单元测试（默认值/字段捕获/深拷贝独立性/往返）。
- **死代码清理**：删除 `crates/tui/src/state.rs`（137 行未编译的重复代码）。
- **回归锁定**：新增 `ctrl_o_is_not_steer` / `ctrl_j_is_not_queue` 测试确保移除的快捷键不会回归。
- **max_steps 清理**：`e2e-glm.sh` 移除 `"max_steps":25`；`steer_followup.rs` 测试名/docstring 去 `resets_step`。
- **编译修复**：4 处 `ChatRequest` 缺 `reasoning_effort` 字段；3 处 clippy `map_or(false, ...)` → `is_some_and(...)`。
- **记忆文档修复**：6 个文件（agents.md / agents/core / agents/session / agents/store / features/index / skill-picker changelog）——去除 max_steps/step 引用、更新 agent 数量/表数/快捷键/测试数。
