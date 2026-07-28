Commit: (working-tree, pre-initial-commit)

# feat(session): autopilot 自驱动 PLAN→ACT→VERIFY 循环

## 背景

普通会话需要人手逐轮驱动：每轮工具执行后回到 idle，等待用户再次输入才能继续。
当任务可被自动分解时（实现→自检→修正），这种「停一步问一次」的节奏拖慢迭代。

autopilot 引入一个**可选的**自驱动循环：在初始任务完成后，runner 把控制权交给
`autopilot::drive`，反复执行 PLAN→ACT→VERIFY 直到 VERIFY 判定任务完成（或达到上限 /
连续 malformed 中止）。**默认关闭**，开启后不影响普通会话的任何既有行为。

## 设计

每个迭代三阶段（`state.rs::ApPhase`）：

- **PLAN** — 切到 plan agent，注入 continuation prompt，跑一个 turn。Plan turn 保留在
  主 transcript 中（合法工作记录）。
- **ACT** — 切到 act agent（上下文沿用，不重置），注入 execute prompt，跑一个 turn。
- **VERIFY** — **隔离的 shadow 一次性调用**：克隆当前 transcript 到一次性快照，让
  small_model 判断「是否还需要更多工作」，解析单个 yes/no，随后丢弃快照。判定交换
  **不写入、不持久化**——主 transcript 绝不被污染。

终止条件（`state.rs::ApOutcome`）：
- VERIFY 回答「no」→ `Complete`。
- 连续 malformed 达 `verify_retries` 次 → `Aborted`。
- 已完成迭代数达 `max_iterations` → `MaxIterations`。

既有 doom-loop（`DOOM_THRESHOLD`）、tool-failure guard、cancel token 仍然约束每个阶段的
单次 run，不会被绕过。

## 变更

### 新增 `crates/session/src/autopilot/`（纯函数式模块，无 class）

| 文件 | 职责 |
|------|------|
| `state.rs` | 纯数据类型：`ApPhase` / `VerifyVerdict` / `ApOutcome` / `ApState`（无 I/O） |
| `decision.rs` | `parse_verdict`（容错 yes/no 解析）+ `should_stop`（纯判定逻辑） |
| `prompts.rs` | PLAN/ACT/VERIFY 各阶段的注入 prompt 构造 |
| `verify.rs` | `verify()`：隔离快照 + small_model 一次性调用 + 重试 |
| `phases.rs` | `run_plan_phase` / `run_act_phase`：切换 agent 并跑一个 turn |
| `mod.rs` | `drive()`：主循环编排，emit `SessionEvent::AutoPilot` 进度事件 |

### 配置（`crates/core/src/config/autopilot.rs`）

新增 `AutoPilotConfig`（拆入子模块以满足行数限制）：

```json
{
  "autopilot": {
    "enabled": false,
    "max_iterations": 10,
    "verify_retries": 3,
    "skill": null
  }
}
```

- `enabled` 默认 `false`——普通会话行为不变。
- `max_iterations`：已完成迭代硬上限（0-based），达上限产出 `MaxIterations`。
- `verify_retries`：malformed VERIFY 裁决的重试次数，耗尽则 `Aborted`。
- `skill`：可选 PLAN 阶段激活的 skill 名，运行时从已发现 skills 解析。

### 事件与展示

- **新 `SessionEvent::AutoPilot { phase, iteration }`**：SSE 序列化为 kind `"autopilot"`。
  所有 5 处 exhaustive match 站点（TUI `chat.rs`、CLI `run.rs`、web、from_sse、sse_data）
  均已覆盖；非穷尽通配站点正确吸收。
- **TUI**：autopilot 阶段反映在状态栏（`autopilot: Plan #0`）。
- **CLI headless**：`print_event` 渲染阶段进度。
- **TUI `/model` 配置表单**：新增 autopilot 字段（enabled toggle、skill 输入）。

### 文件行数合规

- `config.rs` 961→690 行：拆出 `config/autopilot.rs`（63）+ `config/merge.rs`（224）。
- `chat.rs` 804→768 行：拆出 `chat_helpers.rs`（42，summarize/short/block_text）。

## 测试覆盖

新增 24 个测试（workspace 共 1267 passed / 0 failed）。全部用 `MockChatClient`，确定性、
无网络/真模型依赖、无 flaky。

### 单元（9）— `crates/session/src/autopilot/tests.rs`（零 I/O 纯函数）

| 功能 | 测试名 |
|------|--------|
| yes 裁决解析（多变体） | `parse_yes_variants` |
| no 裁决解析（多变体） | `parse_no_variants` |
| 容忍标点/空白 | `parse_tolerates_punctuation_and_whitespace` |
| 垃圾/空输入 → None | `parse_garbage_and_empty_is_none` |
| Complete verdict 停止 | `complete_stops` |
| Malformed verdict 中止 | `malformed_aborts` |
| MoreWork 未达上限继续 | `more_work_under_cap_continues` |
| MoreWork 达上限 → MaxIterations | `more_work_at_cap_is_max_iterations` |
| max_iterations=0 → MaxIterations | `more_work_with_zero_max_is_max_iterations` |

### 集成（11）— `crates/session/tests/autopilot.rs`（MockChatClient + tempdir）

| 功能 | 测试名 |
|------|--------|
| VERIFY=yes → MoreWork 且不污染 transcript | `verify_yes_means_more_work_and_does_not_pollute_transcript` |
| VERIFY=no → Complete | `verify_no_means_complete` |
| 垃圾裁决重试后 malformed | `verify_garbage_retries_then_malformed` |
| 重试直到得到可解析答案 | `verify_retries_until_a_parseable_answer` |
| drive 在 VERIFY=no 时完成完整循环 | `drive_completes_when_verify_says_no` |
| drive 发出阶段进度事件 | `drive_emits_autopilot_phase_events` |
| drive 在持续 malformed 时中止 | `drive_aborts_when_verify_keeps_malformed` |
| max_iterations=1 → MaxIterations | `drive_max_iterations_one_yields_max_iterations` |
| 关闭时不触发 drive | `autopilot_disabled_never_invokes_drive` |
| 经 run + registry 启用并完成 | `autopilot_enabled_via_run_with_registry_completes` |
| doom-loop guard 终止 ACT 阶段 | `doom_loop_guard_terminates_act_phase` |

### 配置契约（1）— `crates/core/tests/config_contract.rs`

| 功能 | 测试名 |
|------|--------|
| autopilot 配置 save round-trip + 深合并 | `autopilot_config_roundtrips_through_save` |

### TUI 配置表单（3）— `crates/tui/src/model_menu/tests/config_tests.rs`

| 功能 | 测试名 |
|------|--------|
| 表单从 config 初始化 autopilot 字段 | `config_form_inits_autopilot_from_config` |
| 空 skill → None | `config_form_empty_skill_produces_none` |
| toggle enabled | `config_form_toggle_ap_enabled` |

### SSE round-trip（既有测试扩充）

`from_sse_roundtrips_all_variants`（`crates/session/src/runner/event.rs`）新增
`SessionEvent::AutoPilot { phase: Plan, iteration: 0 }` 变体，断言由 17→18，覆盖新变体
的 SSE 序列化/反序列化往返。
