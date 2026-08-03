Commit: (working-tree, pre-initial-commit)

# feat(session): plan 模式下后续 prompt 追加只读提醒标签

## 背景

只读的 **plan 模式**用于让模型聚焦生成计划而非直接执行。此前用户的每一条
requirement 都原样进入对话，模型在多轮 plan 对话中容易漂移到 act-mode 的执行动作
（写文件、跑命令）。本次引入 `plan_input_count` 计数器与 `maybe_tag_plan_prompt`
打标签逻辑：**首个 requirement 原样放行**（计数从 0 起，不打标签），**后续每一条**在
末尾追加一段只读提醒，使模型持续意识到"当前在做计划"。

追加的标签为：

```
\n（当前处于只读的 plan 模式，聚焦计划生成）
```

## 变更

### SessionState 字段与打标签逻辑（`crates/session/src/lib.rs`）

- `SessionState` 新增 `pub plan_input_count: usize`（带 doc 注释：当前 plan 阶段已
  提交的 requirement 数；切到 plan 模式或 plan→act 交接后归零；> 0 时后续 plan
  prompt 会被追加只读提醒）。
- `SessionState::new()` 初始化为 `0`。
- `after_handoff()`（plan→act 交接路径）将其重置为 `0`。
- 新增 `pub fn maybe_tag_plan_prompt(&mut self, text: &mut String)` —— 核心逻辑：
  当 `agent.kind == AgentKind::Plan` 且 `plan_input_count > 0` 时向 `text` 追加标签；
  无论是否打标签，只要在 plan 模式就 `plan_input_count += 1`（保证下一个 prompt 知
  道当前这条已经发生过）。act 模式整段为 no-op。
- 新增 `plan_tag_tests` 测试模块（4 条单元测试，见下表）。

### 控制命令分支（`crates/session/src/control_cmd.rs`）

- `SwitchAgent` 分支：当切**到** plan 模式（`name == "plan"`）时，`plan_input_count = 0`，
  开启一个全新的 plan 计数阶段。

### 恢复路径（`crates/session/src/resume.rs`）

- `resume()` 的结构体字面量中初始化 `plan_input_count: 0`（该字段是 runtime-only，不
  落库，故 resume 永远从 0 起）。

### Runner 注入点（`crates/session/src/runner/mod.rs`）

在三处真实 prompt 入口调用 `session.maybe_tag_plan_prompt(&mut text)`，确保所有路径
的后续 requirement 都能被打标签：

1. `run_with_registry()` —— 直连 prompt 路径，在构造 `Message::user_with_images` 之前
   对 `user_text` 打标签。
2. `run_loop()` steer 消费分支 —— 对 steer 文本 `text` 打标签（steer 文本不会被
   `control_cmd::parse` 当作控制命令时才走到这里）。
3. `run_loop()` queue 消费循环 —— 对从队列 `claim_one_queued` 取出的 `q` 打标签。

> 三处均位于"真实 prompt 被记录为 user message"之前；控制命令（`/plan`、`/act` 等）
> 不经过打标签，保持原样 apply。

### 集成测试（`crates/session/tests/plan_tag.rs`）

新增端到端集成测试，驱动真实的 `run` 入口，覆盖 `runner/mod.rs` 三个注入点。使用
`LibsqlStore::open_memory()` + `MockChatClient`（不触达真实 LLM），通过
`MockChatClient::requests()` 校验模型请求体是否携带标签：

1. 直连 prompt 路径 —— `run_with_registry()` 注入点。
2. steer 消费分支 —— turn 边界 steer 提升注入点。
3. queue 消费循环 —— idle 时 `claim_one_queued` 排空注入点。

## 测试覆盖

共 7 条测试（4 单元 + 3 集成）。

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 首个 plan prompt 不打标签（计数从 0 起，调用后变 1） | `plan_first_prompt_not_tagged` | `crates/session/src/lib.rs`（plan_tag_tests） |
| 第二个 plan prompt 打标签（计数 > 0，调用后变 2） | `plan_second_prompt_tagged` | `crates/session/src/lib.rs`（plan_tag_tests） |
| act 模式无论计数多少都不打标签 | `act_mode_never_tagged` | `crates/session/src/lib.rs`（plan_tag_tests） |
| plan→act 交接后计数归零 | `switch_to_plan_resets_count` | `crates/session/src/lib.rs`（plan_tag_tests） |
| 直连 prompt 路径：turn-1 不打标签、turn-2 打标签，模型请求体同步 | `direct_prompt_tags_only_after_first` | `crates/session/tests/plan_tag.rs` |
| steer 提升：kickoff 已推进计数后，turn 边界 steer 文本被加标签 | `steer_prompt_tagged_after_first` | `crates/session/tests/plan_tag.rs` |
| queue 排空：idle 时排出的后续 requirement 回放时被加标签 | `queued_prompt_tagged_after_first` | `crates/session/tests/plan_tag.rs` |

- `cargo test --workspace` → 全部通过。
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 warnings。

## Impact Surface
- **纯增量特性**：仅当 `plan_input_count > 0` 且 `kind == Plan` 时向 `text` 追加后缀；
  act 模式为 no-op，对既有 act-mode 行为零影响。
- 无 API/trait 变更；无存储 schema 变更（`plan_input_count` 为 runtime-only，不落库，
  resume 时重置为 0）。
- `Store` / `ChatStream` 抽象接缝未改动。

## Related Docs
- [agents/session](../../agents/session/index.md) — session 运行时核心、drain 主循环、plan 模式
