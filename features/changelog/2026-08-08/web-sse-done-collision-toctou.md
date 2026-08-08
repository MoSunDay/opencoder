Commit: (working-tree, pre-initial-commit)

# fix(web): SSE `done` 事件碰撞抑制 + agent/model 切换 TOCTOU 回滚

本轮修复 web 层两个独立持久化/竞态缺陷。二者均不触碰 session runner、
store trait 与 CLI，仅落在 `crates/web`。

## Topic 1 — SSE `done` 事件碰撞抑制（P0-1）

### Context

SSE 事件流 `GET /api/sessions/:id/events` 在订阅时用**全部**回放历史事件的
`(kind, data)` 指纹预填去重集 `seen`。而**每个** `done` 事件的负载恒为
`{}`，在 live 线上又总是 `seq: None`。于是任意一条历史 `done` 的指纹
`("done", "{}")` 都会与后来到达的 live `done` **指纹碰撞并被静默丢弃**：

- UI 的 busy spinner 永不复位、send 按钮停在 disabled，连接看似活动却冻结。
- 该 bug 只在历史里存在过 `done`（任意一个）时触发，回放窗口越长越易复现。

### Change Summary（`crates/web/src/api.rs`，`events` handler ~440）

- 订阅时新增 `baseline = state.store.last_event_seq(&id)`，捕获订阅时刻的
  **当前最大已持久化 seq**。
- `seen` 集合的种子化改为只纳入 `seq > baseline` 的回放事件——即真正的
  订阅→查询 overlap 窗口；`seq <= baseline` 的历史 `done` 不再预填进 `seen`。
- 由此 live `done`（`seq: None`、指纹 `("done", "{}")`）不会被历史 `done`
  抑制，按时下发，UI 复位。

### 配套契约修订（`crates/web/tests/bugfix_contracts.rs`，`events_subscribe_first_no_loss_no_dup` ~358）

- 对已持久化事件 E1 的**再广播**改用 `seq: Some(1)`（与其持久化 seq 对齐），
  使 tier-1 的 seq-based 去重（`seq <= max_replay_seq`）能命中。此前用
  `seq: None` 仅走指纹路径，未覆盖到「已持久化事件再广播携带 seq」的真实路径。

### 测试

`live_done_not_suppressed_by_historical_done`（`crates/web/tests/sse_done_collision.rs`）：
两条已持久化 `done`（seq 1 & 2）+ 一条 live `done`，断言收到三帧
`event: done`。旧代码下 live `done` 被去重吞掉、只剩两帧，本测试即失败。

## Topic 2 — agent/model 切换 TOCTOU 回滚（P1-5）

### Context

`POST /sessions/:id/agent`（`post_agent`）与 `POST /sessions/:id/model`
（`post_model`）在写 `update_session` 之前检查 `handle.draining`。但 drain
可在「检查之后、写入之前」的窗口内启动（TOCTOU），此时 meta 变更会落盘，
而 drain 正在运行，造成**持久化状态与执行态不一致**。

### Change Summary（`crates/web/src/api.rs`）

- **`post_agent`（~303）**：在写入前先 `old_agent = state.store.get_session(&id)`
  捕获旧 meta；`update_session` 成功后**再次**检查 `handle.draining`。若写
  入期间 drain 启动，则把 `agent` / `updated_at` 回滚为旧值并返回 409。
- **`post_model`（~382）**：同型模式——捕获 `old_model`、写后复查 draining、
  在 TOCTOU 命中时回滚 `model`。
- 写前 gate（既有的 409 `agent/model switch refused while drain running`）
  保留，写后复查作为竞态兜底。

### 测试（`crates/web/tests/agent_model_toctou.rs`）

- `post_agent_rolls_back_on_toctou_drain_start`：写前 draining=false、写中置
  draining=true，断言返回 409 且 store meta（`agent`）保持旧值未变。
- `post_model_rolls_back_on_toctou_drain_start`：同上，断言 `model` 回滚。

## Impact Surface

- 用户：SSE 连接不再因历史 `done` 冻结 UI；drain 期间 agent/model 切换不会
  留下不一致的持久化状态。
- 不影响：session runner、store trait、CLI。

## Compatibility

不新增数据库字段、迁移或环境变量；`last_event_seq` 为既有 store 方法。HTTP
成功响应格式不变；TOCTOU 命中时新增 409（`agent/model switch refused: drain
started during write`），可由现有错误状态观测，属既有 409 语义的延伸。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| live `done` 不被历史 `done` 抑制 | `live_done_not_suppressed_by_historical_done` | `crates/web/tests/sse_done_collision.rs` |
| 已持久化事件再广播走 seq 去重 | `events_subscribe_first_no_loss_no_dup`（断言强化） | `crates/web/tests/bugfix_contracts.rs` |
| agent 切换 TOCTOU 回滚 + 409 | `post_agent_rolls_back_on_toctou_drain_start` | `crates/web/tests/agent_model_toctou.rs` |
| model 切换 TOCTOU 回滚 + 409 | `post_model_rolls_back_on_toctou_drain_start` | `crates/web/tests/agent_model_toctou.rs` |

## Gate

- 全量回归：`cargo test --workspace` → 全绿（EXIT=0）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- build：`cargo build --workspace` → 编译干净（EXIT=0）。
- 数据与配置：无 schema、迁移、数据库删除、环境变量或公开成功响应变化。

## Related Docs

- [web 模块](../../../agents/web/index.md)
