Commit: (merge, 2db2de5 -> main: sandbox→plan 回退落地 + shellguard 换壳与 task-plan 收敛合流)

# 合并裁决：`refactor/plan-mode-restore-20260901`（2db2de5）合入 main（5d8e3a5）

## 背景

评审 go-live ready 的候选分支把 `AgentKind::Sandbox` 整体回退为 `Plan`（恢复 `/plan` ⇄ `/act` 双模式回切、bash 写拦截保留）；main 侧同期落了 shellguard 换壳（`opencoder-shellguard` crate、分类 cwd 对齐、compat corpus）与 task-plan 两阶段收敛。两线在 41 个文件上重叠、22 个文本冲突。本次合并按既定裁决合流：**main 的结构/新特性保留 + 候选的 plan 命名与语义胜出**。

## 裁决要点

- **模式语义**：`AgentKind::Plan`（serde `alias="sandbox"`）；`/clear_context` **不切换 agent**（候选语义胜出）——main 的 sandbox→act 收敛块删除，孤儿测试重写为 `apply_clear_context_on_plan_keeps_plan`、`plan_clear_context_folds_and_keeps_plan`（tui）、`clear_keeps_plan_gate_then_explicit_act_unblocks`。
- **question 可见性**：plan 恒可见（`latent.rs::is_visible` 的 plan 豁免 + `phantom_question_call_blocked_when_skill_not_active` 断言极性翻转——schema 仍广告、执行门照拦）；act/subagent 需 task-plan skill 解锁（main 结构）。
- **bash_guard**：main 的 shellguard 结构（`classify_with_dir(cmd, workdir)`、`PLAN_ADMITTED`、gate 单点）+ 候选文案（"Blocked in plan mode: … switch to the act agent (/agent act)"，与候选 execute.rs 逐字一致；`Do not retry` 句子按候选删除）。
- **subagent 隐藏**：main 的 `hide_build_subagent(kind, skill_body)` + `valid_subagent_options(hide_build)`/`schema_for(tools, hide_build)` 签名。
- **prompt**：`IN_PLAN_MODE` 环境块取 main 的 release-set 措辞（`writes under /tmp`、`redirects to /dev/null`、`NOT writable`）+ plan 命名。
- **tui**：保留 main 的 `status_chip_fg`（act+task-plan 黄色告警）与 `status_dot(running, anim_tick, chip_fg)`；`agent_chip_fg("plan")` warn。

## 测试基线翻新

- 依赖 sandbox 活名的套件按新契约重写而非删除：`bash_guard_sandbox_mode.rs`→`bash_guard_plan_mode.rs`、`clear_context_sandbox_act.rs`→`clear_context_agent_kept.rs`、`clear_context_sandbox_act_bash.rs`→`clear_context_bash_gate.rs`、`sandbox_subagent_guard.rs`→`plan_subagent_guard.rs`、`legacy_plan_agent_resume.rs`→`legacy_sandbox_agent_resume.rs`（方向反转）。
- **环境无关化**：plain workdir 夹具不再锚定 `CARGO_MANIFEST_DIR`/进程 cwd（仓库可能整个位于被 release 的 /tmp 下）——shellguard compat corpus 改为 `classify_with_dir` 注入 `$HOME` 下的 plain 目录（`bash_guard.rs::plain_dir`），`classify_in_tests.rs`、`bash_guard_plan_mode.rs`、`clear_context_bash_gate.rs` 同步。

## 门禁证据（当次新鲜）

- 全量 `cargo test --workspace --no-fail-fast`：245 套件 / 3778 通过 / 0 失败。
- `cargo clippy --workspace --all-targets -- -D warnings`：0 告警；`cargo build --workspace`：干净。
- `crates/web/spa` vitest：56/56；`scripts/check-spa-drift.sh`：no drift。

## Impact Surface

- 运行时行为：plan/sandbox 命名统一为 plan（legacy 拼写只读兼容）；clear 不再收敛 agent；写拦截文案候选化。
- 不影响：store 行结构、web API 形状、SPA 组件（drift 0）。

## Related Docs

- [候选 changelog](../2026-08-31/plan-mode-restore-from-sandbox.md)
- [agents/session](../../../agents/session/index.md)、[agents/core](../../../agents/core/index.md)
