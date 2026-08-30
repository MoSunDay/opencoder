# 迭代遗留缺陷扫除：latent 解锁顺序性失锁、resume 高亮失明与执行面防线

## 背景

上一轮迭代（skill run-end 清除收敛 + question latent-only + store 冷启动治理 + act chip 高亮）回归全绿（239 suites/3686 tests），但只读深审（session/core + store/tui 双路）确认 10 项遗留缺陷：2 项 P1（复合激活顺序性失锁、resume 镜像未回填）、3 项 P2（幻调用无执行期拦截、旧 shape 无版本库 open 永久失败、消费型 task-plan 全程灰色）、5 项 P3（ClearContext names 残留、run-end 清除失败上报落在 Done 后、扁平 skill 名派生、boot_clock 时钟异常、store 测试盲区）。用户拍板：全修；消费型 `$task-plan` 语义取「新激活也点亮黄」。

## 实现

### crates/session
- **① 解锁顺序性失锁根治**（`tools/latent.rs`）：`unlocked_from_body` 主体改为逐 `> Source:` 路径行提取技能名精确匹配（`skill_name_from_source_line`：目录式取 skills/ 下目录名、扁平取 file_stem），复合 join 两种顺序均解锁；`my-task-plan` 形近名不解锁；body 无 Source 行保留原 500 字符前缀扫描为 legacy fallback（`long_source_path_keeps_question_within_unlock_window` 兼容测试幸存）。
- **③ 执行期二次防线**（`runner/execute.rs` + `latent.rs::latent_execution_allowed`）：泛化执行路径对 latent 工具重检解锁态，未解锁合成 error ToolResult（不 panic、不挂起）。实测必要豁免一处：`question` 且 question_hub 已 attach（TUI 活跃人工通道，`question_flow` 契约钉死）；headless 幻调用与 `ssh_pty` 幻调用一律拒绝。execute.rs 789/800。
- **⑥ ClearContext 双清**（`skill_lifecycle.rs::clear_skill_state` seam）：ClearContext 与 run-end 清除统一双清 `skill_prompt` + `active_skill_names`，guard 改双条件（stale names 不再阻碍清除）。
- **⑦ run-end 清除失败上报可见化**（`skill_lifecycle.rs`）：三次重试全败 Status→`SessionEvent::Error`（compaction 失败先例），断言 Error 落在 Done 之后仍可被事件流消费。
- **⑪ sandbox 统一拦截门（计划外追加）**（`bash_guard.rs::gate` + `runner/execute.rs` 接线）：sandbox 拦截面从「只有 bash」扩为 `SANDBOX_ADMITTED` 白名单（与 sandbox `ToolFilter::Allow` 一致性单测钉死防漂移），幻构调用 `edit`/未广告 MCP 工具不再静默执行落盘（is_error 且工具体不执行）；拒绝话术收敛为 `bash_guard::sandbox_denial`：点名沙箱模式 + read-only + "Do not retry" + 逃生口修正为真实命令 `/act`（根除不存在的 `/agent act`）。非 Sandbox kind 零变化。时间线备注：主条目 19:01 落盘后代码 19:01–19:02 并行会话追加、自有条目 19:10 成文，本节为终账归并。详见 [sandbox-denial-tells-model-readonly](sandbox-denial-tells-model-readonly.md)。

### crates/tui
- **② resume 镜像回填**（`app.rs` 净零行 799/800 + `skill_display.rs::skill_mirror_from_body`）：`initial_skill_state` 的 body 不再丢弃，resume 恢复的 task-plan 首个 idle submit 不熄黄，同根消除 stale-true 冻结窗口与 skill-only submit 失明。
- **⑤ 消费型 `$task-plan` 点亮**（`app_loop.rs` + `skill_persist.rs::plan_highlight_from_consumed_text`）：Queue/Steer 消费边界从事件携带文本重派生，命中 task-plan token 点亮（复合任一命中即亮），未命中保留 revert 语义；theme.rs 注释同步。
- **⑧ 扁平 skill 名派生**（`skill_display.rs`）：parent 为 skills 目录时回退 file_stem。
- **⑨ boot_clock 哨兵**（`boot_clock.rs::mark_candidate`）：时钟读数 ≤0 拒绝 mark，荒谬首帧日志不再可能。

### crates/store
- **④ 旧 shape 无版本库 open 永久失败**（`schema.rs`）：bootstrap DDL batch 前探测 `sessions` 表存在性（唯一 pre-versioning 标记，batch 后无法区分 legacy/fresh）；None-version + preexisting 落入 `migrate(conn, 0)` 全量迁移——无版本行即无从知晓跑过哪些增量，v0 是唯一正确起点，逐版本核对全量跑幂等安全（IF NOT EXISTS / add_column_if_absent 守卫 / 条件回填收敛不动点）。反证：临时禁用 migrate(0) 后 legacy 测试以 `no such column: task_type` 失败。
- **⑩ 盲区测试**（`tests/schema_bootstrap.rs`）：legacy 五表无版本行 open 收敛 + 二次 open 幂等；bootstrap 中段 DDL 失败整体回滚（同 tx 早先产物消失、旧数据原样）修复后重开收敛；`should_checkpoint_wal` 纯函数化钉死 `existed` 双分支。

## 测试清单

| 类别 | 项 | 位置 |
|------|------|------|
| 新增 | `compound_source_sections_unlock_in_both_orders` / `lookalike_user_skill_source_line_unlocks_nothing` / `flat_source_file_stem_unlocks_ssh_pty` / `legacy_body_without_source_lines_still_scans_prefix` / `execution_gate_refuses_calls_the_body_does_not_unlock` | session/tools/latent.rs |
| 新增 | `phantom_question_call_blocked_when_skill_not_active` / `attached_hub_asks_stay_user_visible_without_skill` / `source_line_body_lets_a_real_question_execute` | session/tests/question_tool.rs |
| 新增 | `apply_clear_context_clears_active_skill_names_too`（control_cmd）`stale_names_without_body_still_clear_and_persist` / `clear_failure_visible_after_done_in_run_stream` | session 单元 |
| 新增 | `resume_mirror_backfill_keeps_resumed_task_plan_yellow` / `resume_mirror_backfill_keeps_unskilled_session_gray` / `mirror_backfill_pairs_derived_name_with_body` | tui app_tests/skill_tests.rs、skill_display.rs |
| 新增 | `queue/steer_consumed_task_plan_token_lights_the_chip` 等 4 个消费语义 + 2 个派生单元 | tui app_loop_tests/plan_chip_consume_tests.rs、skill_persist.rs |
| 新增 | `flat_skill_file_derives_name_from_stem` / `directory_style_skill_still_uses_parent_dir` / `mark_rejects_nonpositive_clock_readings` / `stored_zero_would_produce_an_absurd_latency` | tui skill_display.rs、boot_clock.rs |
| 新增 | `legacy_tables_without_version_row_converge_on_open` / `failed_bootstrap_rolls_back_and_reopens_after_repair` / `checkpoint_gate_*` ×3 | store tests/schema_bootstrap.rs、mod.rs |
| 新增 | `denial_names_mode_forbids_retry_points_at_act` / `admitted_set_matches_sandbox_agent_tool_filter` / `gate_passes_non_sandbox_kinds_through` / `gate_refuses_unadmitted_tool_in_sandbox` / `gate_blocks_mutating_bash_in_sandbox` / `sandbox_mode_refuses_unadvertised_tool_without_executing`；修正 `sandbox_mode_blocks_write_command`（断言 `/act`、"Do not retry"、无 `/agent act`） | session bash_guard.rs（lib tests）、tests/bash_guard_sandbox_mode.rs |
| 回归 | 全量 `cargo test --workspace`：241 suites / **3729 passed / 0 failed**（基线 3686，+43；第 ⑪ 项并入后当前树终账复跑，REGRESS_EXIT=0）；clippy `--workspace --all-targets -D warnings` 零警告；fmt 触碰文件零偏差；行数 gate 全过（最大 app.rs 799/800，execute.rs 789/800） | rule-02 门禁 |

## 划出范围

`cargo fmt --check --all` 共 245 处 Diff、去重 95 个违规文件，与本轮 20 个触碰文件零交集（comm 校验，HEAD 既有债务；「89」为过期树旧数）；`skill_context.rs` 复合截断 notice 只指 `paths[0]` 为既有逻辑——均留待独立任务。
