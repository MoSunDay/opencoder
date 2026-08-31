Commit: (working-tree, /task 列表 Codex 化: 活跃排序 + 两行式 + 不变量锁死)

# `/task` 任务列表 Codex 化改造：最近活跃排序 + 两行式行渲染 + 「不新建记录」不变量收口

## 问题与根因

用户两条原话：`/task` 展示的任务内容要改、只记录父 agent、执行 shift+tab 等操作都不要新建 task 记录。调查结论：

1. **列表排序按 `created_at DESC` 而非活跃时间**：clear-context/skill/压缩等只 bump `updated_at`，会话在列表里永远不浮顶——「操作后列表无感知」的直接根源。普通对话轮次连 `updated_at` 都不 bump，活跃语义整体缺位。
2. **行内容噪杂**：`[act]` mode chip、`[skill]` tag、`● N running`、`⊗ N replay pending`、title+preview 同行拥挤，无活跃时长展示。
3. **gating item（shift+tab 新建记录）**：真实复现失败（旧版观感或旧排序导致的错觉）。本轮以 spy-store 测试把不变量锁死；若未来回归，测试直接红。

## 变更

### store：活跃排序与活跃 touch（`crates/store/src/libsql_store/sessions.rs`、`messages.rs`、`types.rs`）
- `list_sessions` 排序改 `MAX(updated_at, created_at) DESC, id DESC`（keyset 翻页 cursor 同键，`(ts|id)` 格式不变，无外部 cursor 生产者，decode 兼容）；`updated_at=0` 的导入行自然回落 `created_at`。parent-only 过滤原样保留。
- **活跃 touch**：`append_chunk_in_tx` 同事务内 `UPDATE sessions SET updated_at = MAX(updated_at, ?)`（取 chunk 内最大消息 `created_at`，单调防乱序回填）——追加消息即活跃，`/task` 沉浮跟随真实使用。`messages::import`（LibsqlStore 覆写）**故意绕过**：批量历史导入不是活跃。
- SQL 瘦身：删除两个 `subagent_tasks` COUNT 子查询与 `SessionListItem::subagent_running/subagent_cancelled` 字段（web/cli 零消费，已核实）。

### tui：两行式行渲染（`crates/tui/src/task_row.rs` 新增、`task.rs` 重写渲染）
- 新模块 `task_row.rs`（107 行，纯函数）：`activity_ts`（updated_at>0 否则回退 created_at）、`relative_time`（now/Ns/Nm/Nh/Nd ago，打开 picker 时捕获一次，不逐秒自刷）、`preview_line`（空回退 `…`，按显示宽度截断防 CJK 撕裂）。
- 会话行两行式：行1 = 相对活跃时长 +（当前会话）`  (current)`；行2 = 首条提交内容截断。**删除** agent chip / skill tag（连同 `skills` 字段、`with_skills`、`skill_tag`）/ ● running / ⊗ replay 徽标；**保留** `+ New task`、`✕ Clear all` 两段确认、fork 模式、`(current)`。弹窗高度按 2 行/会话增长。`task.rs` 785→636 行（<800），`app_loop_actions.rs` 零改动。

### 就地修的两个恢复链路缺口（计划 #5「发现缺口就地修」）
- **wire model 前缀不一致**：`app_task.rs` `/task New` 分支原存 `model_label`（带 provider 前缀全串）作 `session.model`，而 `SessionState::new`/resume 均存裸 `config.model_id()`——同一逻辑模型，两种创建路径发给 provider 的 `model` 字符串不同。收敛为 `new_task_wire_model(config)`（live config 裸 id，session-only `/model` 切换仍带入新会话）。
- **无缓存切换分支漏重置 `history`**：首访会话残留上一会话 composer 上翻历史；补 `*history = Vec::new()`。

### 非目标
不改 fork/subagent 存储结构、title 生成、picker 搜索与 Clear all 语义、web SPA（被动继承新排序，与 CLI/picker 同 store 路径天然一致）。

## gating item 结论

**未复现**：spy-store（`no_session_row_side_effects`）下 `/act_clear_context`、`/act`、`/sandbox`、`/model` 接缝、`$skill` 激活五条链路 `create_session` 调用数恒为 0，父会话集合不变，子会话行不泄漏；bare clear 历史逐条一致。以 6 条测试锁死不变量。

## 测试（功能 → 测试名）

- 活跃排序 / updated_at=0 回退 → `store::list_activity_order::orders_by_recent_activity_not_creation`
- append 活跃 touch 单调 → `store::list_activity_order::append_message_bumps_activity_monotonically`
- cursor 翻页自洽 → `store::list_activity_order::cursor_pagination_follows_activity_order`
- 坏 cursor 报错 → `store::list_activity_order::invalid_cursor_is_an_error`
- import 不冒充活跃 → `store::list_activity_order::import_does_not_masquerade_as_activity`
- parent-only 过滤回归 → `store::task_type_filter::*`（4 条既有）
- 相对时间边界 / 预览截断 → `tui::task_row::tests::*`（6 条）
- 两行结构 / (current) / 空 preview / 弹窗高度 → `tui::task::tests` 新增 4 条 + 既有导航/clear/fork 回归
- 不变量锁死 → `session::no_session_row_side_effects::*`（6 条，spy-store 全链路）
- 切换恢复 A⇄B / 下一 turn 用恢复值 / UI 不串台 → `tui::session_switch_restore::*`（3 条）
- wire model 裸 id → `tui::app_task::tests::new_task_wire_model_strips_provider_prefix`
- 敏感性验证：突变 resume.rs 恢复逻辑，测试变红后字节级还原
- 删除：`store::subagent_status_counts`（字段随 feature 移除）
- **全量回归**：`cargo test --workspace` 全绿（pipefail 下退出码 0）；`cargo clippy --workspace --all-targets -- -D warnings` 0 警告。

## 上线等价验证

`target/release/opencoder session list` 于 /root/opencoder 真实库：最近活跃会话（本任务 + task-plan 在途会话）浮顶，CLI 与 picker 同 store 路径交叉一致。**系统二进制未替换**——工作树尚有并行在途改动（task-plan），换装会把半成品部署出去；等树收敛后按老流程 `cargo build --release` 换装 + TUI 实开四步验收（首行时间差值、shift+tab 后行数不增、/model 切换恢复、CLI 顺序一致）。

## 回滚

无 schema 迁移、无数据回写；revert 本 diff 即回滚。观察项：`sessions.updated_at` 语义由「title/控制命令时间」扩展为「最后消息时间」（MAX 单调，旧值只增不改），web 列表排序被动变化——若 web 端有依赖 created_at 序的消费方出现异常，revert 即恢复。
