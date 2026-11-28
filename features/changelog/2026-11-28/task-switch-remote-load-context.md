# `/task` 切换诊断补齐：远端加载分支的错误上下文与切换链路定向回归

一句话：`/task` Resume / Fork 的切换加载先做远端探测（`remote::load`）再做本地 resume，此前只有本地
分支的错误链带 `task switch resume for session <id>`，远端分支失败时用户只看到底层错误、分不清是哪一步
失败。本轮给远端分支补上对称的 `task switch remote load for session <id>`，并加一条定向测试锁定两条路径
的上下文与底层原因保留；收尾时再追加五条定向回归，把切换链路既有行为锁成测试（切换入口本地 / 远端
加载失败的原子性、压缩会话的 OFFSET 尾部恢复、远端绑定任务的 Fork 拒绝）。不改控制流、不改用户可见
提示；收尾部分只新增测试，生产 Fork 行为未变。

## 背景与决策

- 触发条件（通用）：`load_session_for_switch` 的两条读取分支错误链不对称。远端探测先于本地 resume，一旦
  它失败，裸错误无法区分「远端加载失败」与「本地 resume 失败」，排查只能靠猜。
- 决策：只补上下文（`with_context`），远端探测返回「非远端会话」时仍照旧走本地 resume；不默认、不跳过、
  不改数据；`switch_error` 的渲染与 warn 日志（仅 session id + 错误链）保持不变。
- 与数据损坏的边界（通用陈述）：触发排查的现场切换失败经**只读**检查证实为存储引擎层的数据页损坏
  （多个进程共享同一数据根的场景），不是解码层缺陷。错误上下文只改善定位，**不修复数据**。数据恢复属于
  仓库外运维：候选库已在仓库外完成构建与校验（完整性检查通过、全部会话可读可导出），但**尚未激活**
  （live 库仍被多个进程写入）；根因（多进程 WAL）未修复，多进程共用同一数据根在引擎侧修复前仍是
  Go-Live 阻塞项，需单独决策。

## 行为与实现

- `crates/tui/src/app_task.rs` `load_session_for_switch`：`crate::remote::load(..)` 的结果包一层
  `.with_context(|| format!("task switch remote load for session {id}"))?`；函数 doc 注明远端探测先于
  本地 resume、失败必须带同样的切换 + 会话上下文。
- 不改公共 API / CLI / 依赖 / Ontology 目录；未触碰运行数据库。
- 收尾新增三个测试子模块（经 `crates/tui/src/app_task/tests.rs` 的 `#[path]` 挂载，用例仍在
  `app_task::tests::` 前缀下）：
  - `crates/tui/src/app_task/load_failure.rs`：从事件循环实际调用的 `switch_session` 入口驱动，证明目标
    加载失败（本地 resume / 远端探测）是原子的——当前会话、运行态、显示状态、草稿与快照逐字段不变，不发
    `Quit`；远端探测失败链记为 `task switch remote load for session <id>`，不误记为 resume 上下文。
  - `crates/tui/src/app_task/compacted_switch.rs`：经生产 ts-mirror 包装器走真实切换加载器，折叠头部不可读
    时先合成摘要、再恰好两条 OFFSET 尾部消息；纯加载、头部行保留，回放首块为 Compaction。
  - `crates/tui/src/app_task/fork_failure.rs`：远端绑定任务的 Fork 在拷贝前被拒绝（不落新会话、不改源绑定、
    当前任务不变）；`switch_error` 对 Fork 的整链渲染用合成上下文验证，不表示生产今天会为 Fork 附加
    `task switch fork ...` 上下文。
- 生产代码未随收尾改动：压缩会话加载（纯加载、折叠头部不反序列化）与 Fork 拒绝（拷贝前的守卫）行为与
  此前一致。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 远端分支：kind 为远端 agent 但无绑定运行时的会话切换失败，错误链含 `task switch remote load for session <id>`，且远端层原因仍在链中（不是 resume 上下文） | `load_session_for_switch_error_names_the_session_on_the_remote_path` | `crates/tui/src/app_task/tests.rs` |
| 本地分支既有行为：`load_session_for_switch` 失败错误含 `task switch resume for session <id>` 与 session id | `load_session_for_switch_error_names_the_session` | `crates/tui/src/app_task/tests.rs` |
| `switch_error` 返回完整错误链文本（有 / 无 session id 两种 pick） | `switch_error_keeps_the_cause_chain` | `crates/tui/src/app_task/tests.rs` |
| 切换入口：`switch_session` 层的本地加载失败链（`task switch resume for session <id>`），当前会话 / 运行态 / 显示 / 草稿 / 快照全部保留，不发 `Quit` | `failed_local_resume_at_switch_entry_keeps_current_task_intact` | `crates/tui/src/app_task/load_failure.rs` |
| 切换入口：远端探测失败链（`task switch remote load for session <id>`，不误记为 resume 上下文），原子性与上一行相同 | `failed_remote_probe_at_switch_entry_keeps_current_task_intact` | `crates/tui/src/app_task/load_failure.rs` |
| 压缩会话切换加载：生产 ts-mirror 包装器后的真实切换加载器，折叠头部不可读；先合成摘要、再恰好两条尾部消息；纯加载 / 头部保留，回放首块为 Compaction | `load_session_for_switch_on_compacted_session_loads_summary_then_offset_tail` | `crates/tui/src/app_task/compacted_switch.rs` |
| 远端绑定任务的 Fork：拷贝前被拒绝，不落新会话，源绑定与当前任务不变 | `fork_of_remote_bound_task_is_refused_without_persisting_or_rebinding` | `crates/tui/src/app_task/fork_failure.rs` |
| `switch_error` 对 Fork 的通用整链渲染（合成上下文；不表示生产今天会发出 `task switch fork ...`） | `switch_error_for_fork_keeps_session_id_and_cause_in_the_chain` | `crates/tui/src/app_task/fork_failure.rs` |

- 定向回归：`cargo test -p opencoder-tui --lib app_task::tests::` → 22 passed / 0 failed。
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告。
- 格式：`cargo fmt --check -p opencoder-tui` → 通过。
- 真实 TUI 抽查（非自动化回归，仓库外的临时数据拷贝）：`/task` 选中切换与启动参数恢复均渲染正常、无
  错误闪现。
- 测试范围：本次修改及其直接影响链路；未运行全量测试。

## Impact Surface

- TUI `/task`：远端分支失败的错误链新增 `task switch remote load for session <id>` 前缀；用户可见提示与失败
  处理行为不变（任一失败保留原 worker / 模型 / 历史 / 草稿，不提交半切换状态）。
- 日志：切换失败的 warn 仍只含 session id + 错误链，不含标题 / 提示词 / 消息内容。
- 收尾新增的定向回归只锁定既有行为，不改生产路径：压缩会话加载与 Fork 拒绝的用户可见行为不变。
- 不修复存储层数据损坏；多进程共享同一数据根的使用方式在引擎侧修复前不受支持。
