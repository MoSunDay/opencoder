Commit: (working-tree, post-3320cbb)

# 陈旧二进制导致 ts-origin shift+tab 不交接：根因与发布缺口

## Context

用户报告：plan 模式提交需求并输出计划后，Shift+Tab「切到了 act 没有任何其他
动作」——计划没折叠、没有起回合。本地源码修复早已落地，但现场
持续复现。三路证据闭合后根因锁定为**用户运行的二进制过期**：

1. **现场环境**：用户经 `ts` 启动会话（ts-origin，session 行 `agent=NULL`）。
   会话 `01M0C9S27YR0FH…`（`98d6ae92add52775/opencoder.db`）即 ts-origin：
   agent=NULL、plan_input_count=1、plan_snapshot 已落库——持久态全部正确。
2. **二进制版本**：`/root/.local/bin/opencoder` = `0.1.0 (3320cbb)`（08-19
   08:46 构建），而 ts-origin 修复于 11:42 才落地；**本地 main 领先
   origin/main 4 个提交未发布**。
3. **3320cbb 中的缺陷**：TurnDone(plan) 消费时点重武装含
   `meta.agent == Some("plan")` 合取（ecce7b0 引入）→ ts-origin 行 agent 恒
   NULL → 每次计划回合结束后 `plan_submitted` 被权威地翻成 false → Shift+Tab
   走纯 `SwitchAgent` 分支，与用户实测现象完全吻合。
4. **旁证**：`d3454f…/opencoder.db` 会话 `01M0BSNM…` 留下旧二进制下的降级
   指纹——plan 相位 count=1+快照在库、agent 切 act 但无 handoff_seq、随后
   出现无 user 消息的空 prompt act 回合（SwitchAndStart 门失败降级路径）。

修复本身已在本地 HEAD（去掉 agent 合取、worker 门补 plan_snapshot
子句、handoff 增加 `newest_plan_agent_text` 回退、resume legacy 回填；ecce7b0
实现 steer/queue 消费时点武装）。本条目记录的是**发布缺口**的处置。

## Change Summary

- **`crates/tui/src/app_loop_act_clear_ts_origin_tests.rs`**：将
  ts-origin 变体回归测试从 `app_loop_act_clear_repro_tests.rs` 拆出独立成
  文件：ts-origin 行（agent=None）驱动真实 plan turn（`UiCmd::Prompt` →
  `fold_ui_events` 全量折叠）→ TurnDone(plan) 必须在 agent 列为 NULL 时仍
  重武装 → Shift+Tab 必须产出 `SwitchAndStart`（交接），不得退化为纯
  `SwitchAgent`。
- **二进制重建与原子替换**：从本地工作树 `cargo build --release`（严禁
  `opencoder update`——其默认 clone origin/main=3320cbb，不含修复），按
  `update.rs` 约定 mv 原子替换 `/root/.local/bin/opencoder`（ETXTBSY 安全，
  不 kill 任何在跑实例）。替换后 `opencoder --version` 显示含修复的
  新提交哈希；旧二进制保留为 `opencoder.old-3320cbb` 备查。
- **注意**：替换只影响新启动进程；替换时点在跑的会话仍执行内存中的旧代码，
  需新开会话（或 `--continue`）才生效。

## 测试清单

| 测试 | 层 | 断言 |
|---|---|---|
| `tui app_loop_act_clear_ts_origin_tests.rs::shift_tab_ts_origin_session_hands_plan_forward` | integration | ts-origin 行（agent=None）真实 plan turn 后：计数=1、UI 武装为真、Shift+Tab 发 ResetCancel + SwitchAndStart（用户原场景） |
| `tui app_loop_act_clear_repro_tests.rs::shift_tab_after_real_plan_turn_hands_plan_forward` | integration | 常规（非 ts-origin）plan turn 后 Shift+Tab 交接、transcript 折叠为单条计划消息 |
| `tui tests/handoff_provenance_gate.rs` | integration | 双击/伪 handoff 溯源门：无提交不交接 |
| `session plan_phase / plan_handoff / resume_legacy / steer 相关测试` | unit+integration | 消费时点计数+落库、快照有界、无提交不交接、legacy 回填 |
| 回归：`cargo test --workspace` | regression | 除当时基线既有的 web `client_echo_matches_server_persisted_events` 失败外全绿；合并前已隔离主机配置并实现全绿 |

## Validation（当次实跑）

- `cargo test -p opencoder-tui --lib ts_origin`：1 passed（新增 ts-origin 回归测试绿）。
- `cargo fmt --check`：工作树干净（纯格式化、与逻辑变更分离）。
- 二进制核验：`opencoder --version` 由 `0.1.0 (3320cbb)` 更新为含修复的新哈希。
- 人工冒烟（需用户执行，新开会话）：`ts` → shift+tab 进 plan → 提交需求 →
  计划输出 → shift+tab ⇒ transcript 折叠为仅计划卡片 + act 立即执行；反向：
  plan 下不提交直接 shift+tab ⇒ 纯切换、上下文不动、不起回合。

## Risks / Follow-ups

- 本地 4+2 个领先提交是否 push origin 由用户另行决定，本任务不 push。
- web `POST /handoff` 提交溯源门加固另开任务（范围外）。
