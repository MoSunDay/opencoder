Commit: (working-tree, plan 模式残留终清 + quickstart 复合命令纠偏 + 两个加固测试)

# plan 模式残留终清：文档纠偏、测试前提修正、死文件删除与复合契约钉死

## 背景

review 在「plan 模式已删除、规划职责由 act 下 task-plan skill 承接」的语义模型下盘点出残留清单：行为契约（ClearContext 不切 agent、/sandbox⇄/act 纯切换、sandbox 只读防线）全部有测试护栏且全绿，扣分项集中在收尾面——用户文档与实现矛盾、一处语义性陈旧测试前提、两个未挂载死测试文件、若干注释级误导措辞。

## 实现

- **D0 死测试文件删除**：`crates/session/src/bash_guard_tests.rs`、`bash_guard_security_tests.rs` 不再被任何 `#[path]` 挂载，断言与 shellguard 新策略矛盾（如 `find /tmp -delete` 期望拦截 vs 释放集放行）——删除，防误挂载/误导读者；`bash_guard_compat_tests.rs` 头注释同步改写为「冻结换壳前行为兼容面」。
- **D1 quickstart 纠偏**（中英）：删掉教用户 `/plan <内容>` 复合提交的行（TUI 路径会把 `/plan` 按普通文本送入 runner），替换为现行真实命令：`/sandbox <内容>` 复合（切换后尾文本作 prompt）与 `/act_clear_context <内容>` 复合（折叠上下文、agent 保持不动、别名 `/clear_context`）。
- **D2 测试前提修正**：`crates/core/src/config/tests.rs` `parent_flag_covers_every_primary_agent` 的 `["act", "plan", "command"]` → `["act", "sandbox", "command", "workflow"]`（依 agent.rs：Primary = act/sandbox/command/workflow，explore/build 为 Subagent）；editable-key 探针值 `"plan"` → `"act"`。
- **D3 注释/命名残留收敛**：`tools/question.rs` fixture `agent:"plan"`→`"act"`；`bash_guard_sandbox_mode.rs` `plan_mode_allows_subshell_fd_merge`→`sandbox_mode_allows_subshell_fd_merge` + 注释；`sandbox_subagent_guard.rs` 断言消息与 doc 注释（删掉不存在的 `AgentKind::Plan` 虚构与旧测试名，改为引用真实 `sandbox_mode_blocks_build_subagent`）；`skill_contract.rs` "task-plan runs in plan mode" 注释改为「question 由 task-plan skill 解锁（latent 500 字符窗门控），act/sandbox 统一澄清通道」；TUI 三处 "plan-mode" 措辞 → "plan editor"/"skill-gated"（plan 编辑器是现行功能，非模式）；`agent_switch_roundtrip.rs` 补 total-noop 前置条件注释（armed skill 下裸 `/act` 会清 skill，见 skill_early_exit_clear.rs）。
- **D4 记忆口径**：`agents/session/index.md` 控制命令测试行——同 agent 重切 no-op 标注无 armed skill 前提；追加 `tests/clear_context_skill_compound.rs` 条目。
- **D5 加固测试 ×2**：
  - `tools/latent.rs::long_source_path_keeps_question_within_unlock_window` + review 对照——≥240 字符真实深 HOME 下 500 字符窗不失锁。调查结论：解锁键是窗口内的 **skill 名 token**（写在 Source 路径里）而非 `question` 关键词，真实失效上界 HOME ≳470 字符，当前安全。
  - `tests/clear_context_skill_compound.rs`（新文件，343 行）——钉住 `/act_clear_context $task-plan <尾文本>` 复合链：apply 清 skill（内存+store）→ 尾文本消费时 `$token` 经 `extract_skill_tokens` 即时发现重新武装（`[skill loaded]` 入 payload、token 从落库 prompt 剥离）→ run-end 清除压回 NULL；并钉住反向契约：**无 `$token` 的尾文本永不重新武装**。

## 测试清单

| 类别 | 项 | 位置 |
|------|------|------|
| 新增 | `long_source_path_keeps_question_within_unlock_window` / `..._review_body_still_unlocks_nothing` | tools/latent.rs（lib 19 passed） |
| 新增 | `clear_context_clears_armed_skill_and_dollar_less_tail_does_not_rearm` / `dollar_tail_rearms_task_plan_then_run_end_clears_store` | tests/clear_context_skill_compound.rs（2 passed） |
| 改名 | `sandbox_mode_allows_subshell_fd_merge`（断言未动） | tests/bash_guard_sandbox_mode.rs（6 passed） |
| 修正 | `parent_flag_covers_every_primary_agent`（primary 清单真实化） | config/tests.rs（lib config 109 passed） |
| 注释 | skill_contract question 门控注释（断言未动） | tests/skill_contract.rs（23 passed） |

## 全量回归

- `cargo test --workspace --no-fail-fast`：**3686 passed / 0 failed，exit 0**（239 个 test result 面全部 ok）。
- 扫尾定向（覆盖最后一轮注释/清单编辑触达目标）：agent_switch_roundtrip 2 + clear_context_skill_compound 2 + control_cmd 9 + bash_guard_sandbox_mode 6 + sandbox_subagent_guard 3 + core lib config 109 + skill_contract 23，全部 0 failed。

## 遗留（非本轮）

- web SPA 无 task-plan 激活指示（TUI act chip 黄色高亮的对等物）——功能缺口，入 backlog。
- `opencoder_shellguard::classify` 取进程 cwd 而非 session working_dir 的隐性耦合——相对路径目标在多会话 daemon 下按进程 cwd 求值；cwd 不在释放集、相对写一律拦截，风险低，留观。
