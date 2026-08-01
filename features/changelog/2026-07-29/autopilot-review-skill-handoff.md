# refactor(autopilot): 固定 review skill + ACT handoff 重置 transcript

## 背景

autopilot 的 PLAN 阶段此前通过 `config.autopilot.skill`（`Option<String>`）配置激活哪个 skill，
ACT 阶段则直接沿用 PLAN 的完整 transcript 并注入 `execute_prompt`。

问题：
- skill 名称需要手动配置，增加了用户认知负担。
- ACT 阶段沿用 PLAN 全部上下文，plan agent 的只读探查记录对 act agent 是噪音。
- `execute_prompt` 与 review 输出信息重叠。

## 设计

### PLAN 阶段：固定 review skill

`maybe_activate_skill`（读 `config.autopilot.skill`）→ `activate_review_skill`（固定 discover `"review"`）。
review skill 的职责是评估当前状态并输出 gaps，即为 ACT 阶段的 plan。

### ACT 阶段：handoff 重置 transcript

`run_act_phase` 开头调用 `plan_handoff::handoff(session, "")`：
- **成功** — transcript 重置为 `[HANDOFF_PREFIX + review输出]`（1 条 synthetic user message，
  已含执行指令）；持久化 `handoff_seq`/`handoff_plan` 到 store；发出 `TranscriptReset` 事件；
  `set_skill(None)` 清除 skill；切到 act agent；直接 `run_loop`（不注入 `execute_prompt`）。
- **失败**（无 assistant 文本，极端情况）— fallback：`set_skill(None)` + 切 act agent +
  注入 `execute_prompt` + `run_loop`。

`drive()` 在返回 `Complete` 前调用 `session.set_skill(None)` 复位。

### 循环流程

```
启动任务 → run_loop 停止
  → PLAN (plan agent + review skill): 评估输出 gaps
    → handoff: transcript = [HANDOFF_PREFIX + review输出]
    → set_skill(None), 切 act agent
    → ACT: 执行 gaps plan
    → VERIFY: 目标是否达成?
       → 是 → Complete
       → 否 → PLAN → 循环
```

## 变更

### 移除 `AutoPilotConfig.skill` 配置字段

| 文件 | 改动 |
|------|------|
| `crates/core/src/config/autopilot.rs` | 删 `skill: Option<String>` 字段 + serde 属性 + Default + `merge()` 分支 |
| `crates/core/src/config/merge.rs` | 删 `has_editable_key` 中 `contains_key("skill")` |

### autopilot phases 重构

| 文件 | 改动 |
|------|------|
| `crates/session/src/autopilot/phases.rs` | `maybe_activate_skill` → `activate_review_skill`（固定 discover `"review"`）；`run_act_phase` 改为先调 `plan_handoff::handoff`，成功则持久化 + emit `TranscriptReset` + 不注入 `execute_prompt`，失败则 fallback |
| `crates/session/src/autopilot/mod.rs` | `drive()` 退出 Complete 前调 `session.set_skill(None)` |

### TUI 配置表单移除 ap_skill

| 文件 | 改动 |
|------|------|
| `crates/tui/src/model_menu/config_form.rs` | 删 `ap_skill_input` 字段、`ConfigField::ApSkill`、ORDER 条目、new/build_patch/key-handling |
| `crates/tui/src/model_menu/view.rs` | 删 `ap skill:` field_line |
| `crates/tui/src/model_menu/patch.rs` | 删 `ap_skill` 字段 + JSON `"skill"` 键 |

### 旧 config 兼容

serde 未设 `deny_unknown_fields`，旧 config 中残留的 `"skill"` 键被静默忽略。

## 测试清单

### 集成（2 新增）— `crates/session/tests/autopilot.rs`

| 功能 | 测试名 |
|------|--------|
| ACT handoff 重置 transcript + 清除 skill（正常路径） | `act_phase_handoff_resets_transcript_and_clears_skill` |
| ACT handoff 失败 → fallback 注入 execute_prompt（边界路径） | `act_phase_fallback_injects_execute_prompt_when_plan_has_no_text` |

helper `autopilot_config` 去掉 `skill` 参数。

### 配置契约 — `crates/core/tests/config_contract.rs`

`autopilot_config_roundtrips_through_save` 去掉 `skill` 断言。

### TUI — `crates/tui/src/model_menu/tests/config_tests.rs`

删 `config_form_empty_skill_produces_none`；`enter_chains` 链从 ApMaxIter → Save；
`config_form_inits_autopilot_from_config` 去掉 skill 断言。

## Verify

- `cargo build --workspace` — 通过
- `cargo clippy --workspace --all-targets -- -D warnings` — 通过（零 warning）
- `cargo test --workspace --no-fail-fast` — 1299 passed, 0 failed, 0 ignored
