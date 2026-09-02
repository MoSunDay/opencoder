Commit: d4714fd (working-tree, re-clear 保留已持久化交接边界——fold 不再覆写 sentinel)

# 二次 ClearContext 保留已持久化交接边界（Plan 卡不再被哨兵覆写清空）

## 背景

shift-tab-repress-confirm（armed 态第二次 Shift+Tab 立即确认）上线后大幅提高了
ClearContext 二次触发的概率，暴露出 `fold_to_continuity_seed` 一直存在的覆写缺陷：
当 transcript 只剩 synthetic 消息（二次 clear、resume 后再 clear、act 未产出文本时
clear），`handoff::last_assistant_text` 提取必然落空，`None` 分支把
`session.handoff_plan` **无条件覆写为 `CLEAR_CONTEXT_SENTINEL`**——UI 重建 Plan 卡时
精确过滤哨兵（`replay.rs`），已保留的 plan 指令从卡片与模型两侧同时消失，屏幕上
什么都不剩。持久化时序无竞态（`persist_clear` 先于事件 emit）。

## 修复

`crates/session/src/control_cmd.rs::fold_to_continuity_seed` 的 `None`（无 assistant
文本）分支改为：

- `handoff_plan` 已持有**非哨兵、非空**边界时原样保留：
  - 种子边界（`<<OPENCODER_CLEAR_SEED>>` 前缀）→ 重建 `seed_message(clear_seed_text(prev))`；
  - 指令 display → 重建 `handoff_message(prev)`（与 `resume.rs` 三分支重建同款，
    内存 transcript 与 resume 自洽）；
  - 非 sentinel → `is_clear_context_handoff=false` → `handoff_pending=true`，
    模型继续执行计划/种子，语义不变；
- 仅当从未保留过任何边界（哨兵或 None）才落 fresh-start + 哨兵（全新空会话行为
  不变）；compaction 已把 `handoff_plan` 清空（`after_compaction`），陈旧边界不会
  误保留。

已知边界（本次不处理）：act 已产出文本后再 clear，`newest_work_text` 会把 act 最后
输出当 brief 覆盖卡片，信息不足以区分 plan 产出，需单独设计。

## 测试（rules/01）

- `crates/session/tests/clear_context_reclear_preserves.rs`（新增，3 用例）：
  - `plan_directive_survives_reclear_and_still_executes`：plan→act 交接后二次
    clear（transcript 仅剩 synthetic 指令）→ `handoff_plan` 保持原 display、
    messages 为 `handoff_message(prev)`、drain 模式 LLM turn 仍执行、resume 重建
    同款指令；
  - `seed_boundary_survives_reclear_and_still_executes`：种子边界再 clear → 种子
    保留、LLM turn 执行、resume 重建同款种子；
  - `blank_boundary_reclear_stays_blank_without_llm`：全新空会话 re-clear 仍哨兵、
    零 LLM 调用（`plan_sentinel_clear_stops_without_llm` 语义不变）。
- `crates/tui/src/session_ui/handoff_card_tests.rs`（新增，3 用例）：`rebuild_after_reset`
  后 Plan 卡按指令 display / 剥离标记的种子文本渲染，哨兵永不渲染为 Plan 卡。
- 既有 `clear_context_agent_kept.rs`（5，含 `plan_sentinel_clear_stops_without_llm`）/
  `clear_context_toggle_regression.rs`（4）/ `clear_context_regression.rs` /
  `handoff_resume.rs` / `handoff_clears_compaction.rs` / `plan_act_dup_check.rs` /
  `clear_context_skill_compound.rs`（2，clear×skill 交叉）全部通过。
- 本轮验证读数（d4714fd 工作树）：`cargo test -p opencoder-session` 全量
  788 通过 / 4 失败，4 例全部属并行 skill_context 重构中态（`skill_body_injection`
  2 例、`skill_tail_cleared_after_run_end` 1 例、`skill_mid_run` 1 例），与本修复
  无交集，归该工作流自身 rules/02 门禁收口。
- TUI：主工作树暂存的 sidecar 重构中态致 tui lib 暂不可编译（7 个既有错误，均在
  `app.rs`/`app_helpers.rs`/`app_loop_actions.rs`/`app_task.rs`，与本修复无关）；
  以 HEAD 临时 worktree 仅叠加本修复两文件隔离验证：`handoff_card_tests` 3/3、
  `session_ui` 模块 32/32、tui lib 1551 用例编译通过。
- 全仓 `cargo test --workspace` 统一回归待并行 skill/sidecar 工作流收口后补跑。
