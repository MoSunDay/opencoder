Commit: (working-tree, pre-initial-commit)

# Bugfix Sweep Wave 3 — Low-Severity Defects + Flaky Test Root-Cause Fix

## 背景
第三波系统化 bug 修复，聚焦 low-severity 缺陷与一个长期困扰 CI 的 flaky test 根因修复。涵盖 8 个 crate、~21 项修复 + 1 项 flaky test 根因消除。

## 变更

### llm (3 fixes)
- **`crates/llm/src/request.rs`** + **`crates/llm/src/client.rs`**：`temperature` 字段从 `f32` 改为 `f64`，消除 `0.7_f32 as f64 == 0.6999999880790710` 的 JSON 序列化精度丢失
- **`crates/llm/src/sse.rs`**：SSE 解码器现在按规范将同一帧内多个 `data:` 行用 `\n` 拼接为单个事件（此前每行作为独立事件输出）
- **`crates/llm/src/client.rs`**：移除 `handle_event()` 调用上的死代码 `.map_err(OnceError::Connect)?`（该函数不可失败，map_err 永远不触发）

### core (4 fixes)
- **`crates/core/src/config/merge.rs`** + **`crates/core/src/config/autopilot.rs`**：`tail_turns`/`max_iterations`/`verify_retries` 的 u64→u32 转换加 `.min(u32::MAX as u64)` clamp，防止 ≥2³² 静默回绕
- **`crates/core/src/config.rs`**：doc 注释中 tool_guard 默认阈值从错误的 "5" 更正为实际的 "20"
- **`crates/core/src/tool.rs`**：`head_tail_lines` 加 `saturating_sub` 防 usize 下溢 + 守卫（行数不足 head+tail 时返回全文）
- **`crates/core/src/lib.rs`** + **`crates/core/src/config.rs`**：新增 `scoped_config_home()` / `ScopedConfigHome` — 线程级配置目录注入，为 TUI flaky test 修复提供基础设施

### store (5 fixes)
- **`crates/store/src/types.rs`**：`SubagentStatus::parse()` 补 `"unknown"` 臂，修复 Unknown→"unknown"→Running 的 round-trip 不匹配
- **`crates/store/src/libsql_store/messages.rs`**：`append()` 改为委托 `append_many()`（单条消息入事务），消除 INSERT+SELECT MAX(seq) 的跨进程竞态
- **`crates/store/src/libsql_store/sessions.rs`**：搜索过滤 LIKE 转义 `%`/`_`/`\` + `ESCAPE '\'`，防用户输入充当通配符
- **`crates/store/src/libsql_store/sessions.rs`**：畸形分页游标不再静默忽略，改为返回错误
- **`crates/store/src/jsonl.rs`**：`create_dir_all` 错误不再 `.ok()` 吞掉，改为 `.context()?` 传播

### web (4 fixes)
- **`crates/web/src/api.rs`**：`post_subagent_steer` 新增 `task.parent_session_id != id` 校验，拒绝跨 session 的 task steer
- **`crates/web/src/api.rs`**：`post_interrupt` 对 idle handle 返回 `ok:false`（检查 `draining` flag），不再误报成功
- **`crates/web/src/auth.rs`**：token 比较改用恒定时间 `ct_eq()`，消除时序侧信道
- **`crates/web/src/api.rs`**：`get_events` 先验证 session 存在性，不存在返回 404（不再永久挂起 SSE）

### session (2 fixes)
- **`crates/session/src/runner/subagent.rs`**：flusher `await` 加 30s 超时（匹配 resume.rs 的模式），防 DB flush 任务卡死无限阻塞
- **`crates/session/src/runner/event.rs`**：`from_sse` 的 `seq` 解析从 `.unwrap_or(0)` 改为 `?`，拒绝畸形 seq 而非静默替换为 0

### tui — flaky test 根因修复 (核心改动)
- **`crates/tui/src/app_loop_tests/mod.rs`** + **`crates/tui/src/model_menu/tests/common.rs`** + **`crates/tui/src/model_menu/tests/provider_tests.rs`** + **`crates/tui/src/worker/tests_reload.rs`** + **`crates/tui/src/local_cmd.rs`** + **`crates/tui/src/skill_persist.rs`** + **`crates/tui/tests/` (4 files)**：彻底消除所有 `std::env::set_var`/`remove_var` 调用（线程不安全 UB），替换为 `scoped_config_home()` 线程级注入或显式参数传递。此前三个 queue-scroll 测试在全量并行下偶发失败——根因不是测试自身逻辑（它们是纯函数无副作用），而是同进程内兄弟测试的 `set_var` UB 导致进程级崩溃，三个纯测试作为附带损害被报失败
- **`crates/tui/src/app_helpers.rs`** + **`crates/tui/src/app_task.rs`**：2 处 `.lock().unwrap()` → `.lock().unwrap_or_else(|e| e.into_inner())`，与其他 3 处对齐，消除 mutex poison panic 风险

### cli/client (4 fixes)
- **`crates/cli/src/ts/actions.rs`**：`start_new` 预检 tmux session 是否已存在，已存在则友好报错而非让 tmux 报晦涩错误
- **`crates/cli/src/session_cmd.rs`**：`session show` 非 JSON 路径补 session 存在性检查，不存在时 `anyhow::bail!` 而非静默无输出
- **`crates/client/src/remote.rs`**：`run_stream` 非 2xx 响应读取 body 并包含在错误信息中，与 `ensure_ok` 对齐
- **`crates/cli/src/ts/env.rs`**：`which_tmux` 补执行位检查（`mode() & 0o111 != 0`），非可执行文件不再被误报为找到

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| llm: temperature 序列化 round-trip | `request.rs::tests`（3 tests） | `crates/llm/src/request.rs` |
| llm: 多行 data 拼接 | `sse.rs::tests`（4 tests） | `crates/llm/src/sse.rs` |
| store: SubagentStatus Unknown round-trip | `subagent_status_counts.rs` | `crates/store/src/types.rs` |
| web: post_interrupt idle `ok:false` | `bugfix_contracts.rs` | `crates/web/src/api.rs` |
| web: draining=true 构造 | `web_contract.rs` | `crates/web/src/api.rs` |
| tui: 全量并行零 flaky | 832 测试 3× 连续运行零失败 | `crates/tui/` |

- 全量回归：`cargo test --workspace` → 1723 passed (0 failed)
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：所有文件 ≤ 800 行

## Impact Surface
- 用户可感知：token 比较安全性提升（auth）；不存在的 session/webhook 不再挂起；畸形 cursor 报错而非静默降级；temperature JSON 精度正确
- 不影响：Store trait 接缝、ChatStream 抽象、CLI 命令接口

## Related Docs
- [agents/llm](../../agents/llm/index.md)
- [agents/store](../../agents/store/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/core](../../agents/core/index.md)
- [既有 wave1 changelog](./bugfix-sweep-wave1-high.md)
- [既有 wave2 changelog](./bugfix-sweep-wave2-medium.md)
