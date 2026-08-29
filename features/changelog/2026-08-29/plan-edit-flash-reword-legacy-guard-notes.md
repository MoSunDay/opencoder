Commit: (working-tree, plan 卡片编辑器闪烁文案纠偏 + 双模式删除后的文档真值化)

# plan 卡片编辑器闪烁文案纠偏 + switch_mode 删除后的误导文案清理（含「有意保留」清单）

## 背景

64a4878（2026-08-28，[sandbox 模式替换 plan/act 双模式](../2026-08-28/sandbox-mode-replace-plan-act.md)）删除了 `switch_mode`(Ctrl+T)/`switch_mode_clear`(Alt+Tab)/`switch_mode_keep`(Ctrl+Shift+Tab) 三个绑定与整个双模式系统，但 TUI 残留了三处误导：plan 卡片编辑器进入时的闪烁提示写成 "→ plan mode"（plan 早已不是模式，进入编辑器也不是模式切换）、`pre_key_intercept` 的 doc comment 仍声称 "Ctrl+T is a pure act<->plan mode toggle (see handle_key)"（绑定已不存在，所指分支亦已消失）、内部命名 `plan_mode`/`plan_label` 与已删模式系统同形，易诱使未来迭代顺着「模式」语义误改误删。本轮只做文案与命名真值化，**零行为变更**。

## 实现

- **闪烁文案**：`app_loop.rs::enter_plan_edit` 的 "→ plan mode" → "→ edit plan"；`frame.rs::is_warn_flash` 判定分支同步换字面量——warn 色语义不变（plan 编辑器闪烁仍属警示黄一族）。
- **doc comment 真值化**：`app_helpers.rs::pre_key_intercept` 删除失实的 Ctrl+T 双模式切换说明，改为「switch_mode 系列绑定已随双模式移除，未绑定 chord 原样透传 `handle_key`」，与函数真实行为一致。
- **内部 rename（纯改名，零行为变更）**：`render.rs` + `frame.rs` 的 `plan_mode`/`plan_label` → `plan_edit_mode`/`plan_edit_label`——它们承载 plan 卡片编辑器（overlay）的 mode label（composer 提示、标题、copy-mode 净化判定入参），命名去「agent 切换模式」之歧义。
- **测试同步（仅注释/字面量，全部断言保留未弱化）**：
  - `app_helpers_tests/mod.rs`：ctrl_t 透传测试 reword 为 `legacy_unbound_chord_passes_through_ctrl_l_clears_ctrl_f_redraws`——守卫「未绑定 chord 原样透传 `handle_key`」的语义（Ctrl+L 折叠清理 + Ctrl+F 强制重绘臂逐条保留）。
  - `frame/tests.rs`、`render_tests/chips.rs`、`render_tests/composer.rs`：钉住旧文案的断言字面量/注释跟随 "→ edit plan"（chips 侧 `is_warn_flash("→ edit plan")` warn 黄用例、composer 侧 plan label 渲染断言等均未删除）。

## 有意保留（非残留）

以下字样/命名**看似**双模式残留，实为各自功能必需，未来迭代不得顺手清理：

1. `crates/core/src/config/keymap.rs` legacy 测试 fixture 中的 `"switch_mode": "ctrl+t"`（连同 `switch_mode_clear`: "alt+tab"、`switch_mode_keep`: "ctrl+shift+tab"）：守卫旧用户 keymap JSON 的 serde 容忍（防止 re-introduce `deny_unknown_fields` 导致旧配置加载失败），并断言 `cfg.get("switch_mode").is_none()`（字段确实已不存在、不落到绑定表）。
2. `ChatBlock::Plan` 与 `chat.rs::PLAN_HEADER`（`╶─ plan ─╴`）：plan 卡片 markdown 功能的块类型与标题行，服务于 task-plan 技能产出的 plan 卡片渲染（含 `copy_mode/clean.rs` 的 PlanHeader 剥离路径），与已删的 agent 模式系统无关。
3. `keymap_menu/help.rs` 反广告断言：`assert!(!HELP.contains("切换 plan / act"))`（及 `!HELP.contains("仅切换模式")`）——防止帮助文案重新宣传已删除的模式切换，属防回归护栏，不是待清理文案。
4. `app_loop_actions.rs` 的 `format!("→ {name} mode")`：泛化闪烁模板，`name ∈ {sandbox, act}`（`/sandbox`、`/act` 真实模式切换的 `→ sandbox mode` / `→ act mode`），由 `frame::is_warn_flash` 的 "→ sandbox mode" 分支消费；与已删除的 "→ plan mode" 无关。
5. `theme.rs::agent_chip_fg("plan")` → ACCENT 兜底：映射仅特判 `sandbox` → warn 色，其余 agent 名（含旧库 resume 读回的 legacy "plan" agent）落 ACCENT 兜底，保证历史 transcript 的 chip 不缺色不 panic。

## 验证记录

- `grep -rn "→ plan mode" crates/tui` → 0 命中（零残留，当次实跑）。
- `cargo test --workspace` → 236 套件 / 3345 passed / 0 failed（修复后全量，当次实跑）。
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零告警。
- 行数合规（迭代 ≤800，本轮零新增文件）：app_loop.rs 669 / frame.rs 150 / app_helpers.rs 761 / render.rs 788 / app_helpers_tests/mod.rs 730 / frame/tests.rs 29 / render_tests/chips.rs 386 / render_tests/composer.rs 309。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| plan 编辑器闪烁文案「→ plan mode」→「→ edit plan」（`is_warn_flash` 判别分支同步换字面量） | `warn_flash_hue_covers_sandbox_plan_and_clear_guard` | `crates/tui/src/frame/tests.rs` |
| 闪烁 chip 渲染接受「→ edit plan」（warn 黄语义、双色规则不变） | `mode_flash_chip_two_colour_only_for_definite_switch` | `crates/tui/src/render_tests/chips.rs` |
| `pre_key_intercept` 失实 Ctrl+T doc 移除 + 未绑定 chord 原样透传守卫 | `legacy_unbound_chord_passes_through_ctrl_l_clears_ctrl_f_redraws` | `crates/tui/src/app_helpers_tests/mod.rs` |

- `plan_mode`/`plan_label` → `plan_edit_mode`/`plan_edit_label` 为纯改名零行为变更：既有断言全部保留未弱化，无新增/删除测试。
- 全量回归：`cargo test --workspace` → 233 套件 / 3317 passed / 5 failed（tip `0108e5c` 内容树一手实跑，`/tmp/oc_r2_ws_test.log`；smoke/runner_control 4 项 solo 复跑 PASS 属负载饥饿，runner_cancel target 1 项 pre-existing flake——base `64a4878` 同 FAILED）。套件数与本条目轮次记录（236/3345）的差异来自并行在途任务面，本轮触达面内无测试删除；本条目自身轮次的当次实跑数字见上「验证记录」。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（CLIPPY_EXIT=0）。

## 关联

- 前因：[sandbox 模式替换 plan/act 双模式](../2026-08-28/sandbox-mode-replace-plan-act.md)（64a4878，switch_mode 绑定删除处）。
- 相关：[/act_clear_context canonical 更名 + Shift+Tab 倒计时确认防护](clear-context-countdown-guard.md)（同日；`is_warn_flash` 的 "→ clear" 族出自该轮，本轮 warn 判别三族 sandbox/edit plan/clear 并存）。
