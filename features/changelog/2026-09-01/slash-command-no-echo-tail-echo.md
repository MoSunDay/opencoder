Commit: (working-tree)

# slash command 不回显不进 context；复合尾参（其他需求）回显并进 context

## 背景

控制命令（`/act`、`/plan`、`/act_clear_context`/`/clear_context`）在 runner 层早已「bare 不 record、compound 只 record 尾参」——命令 token 本身从不进入 LLM context。但**回显侧**仍按 raw 原文走：

- queue/steer 消费事件（`QueueConsumed`/`SteerConsumed`）携带**raw 全文**（含 `/plan` 前缀），TUI/web/CLI 据此回显——命令 token 被当作用户话术显示，与「不进 context」的事实相悖；
- TUI idle 复合提交（`/plan review`）走普通 prompt 分支，`push_user` 回显整条 raw；
- `/act_clear_context` 倒计时 fire 路径则相反——连尾参都不回显，fresh context 里跑的需求在 transcript 上不可见。

用户要求：slash command 不回显、不进 context；同一提交里的其他需求（复合尾参）要回显、也要进 context。

## 实现

- **`crates/session/src/control_cmd.rs`**：新增单一事实源 `consumed_echo_text(input) -> Option<String>`——compound 返回尾参（恰好是 `record_compound` 落进 context 的文本）、bare 返回 `None`（inline 应用、零记录、零回显）、非命令原样返回。`lib.rs` 再导出。
- **`crates/session/src/runner/drain.rs` / `steer.rs`**：queue/steer 消费事件的 `text` 改为 `consumed_echo_text(...).unwrap_or_default()`——事件从「raw 回显」收敛为「model-facing 回显」。`event.rs` 两处字段 doc 同步。
- **`crates/tui/src/app_loop.rs`**（QueueConsumed）与 **`chat.rs`**（SteerConsumed）：显示层对事件文本再走一次同一归一化（存量持久化事件携带 raw 前缀也保持正确），事件文本为空时回退本地镜像并同规则归一化——bare 命令消费时**不产生任何 user block**（镜像行仍按 seq 即时收缩）。
- **`crates/tui/src/app_helpers.rs::push_user`**：拆出 `echo`（transcript 上屏文本）与 `history_text`（arrow-up 原始输入）双参；`first_prompt`（标题源）继续以 raw 判定，slash 前缀排除语义不变。5 处调用点同步。
- **`crates/tui/src/app.rs`**：idle 普通分支对 `clean` 归一化——`/plan review` 只回显 `review`，ctx 估算仍按 `clean`。
- **`crates/tui/src/app_loop_actions.rs::fire_clear_confirm`**：idle fire 前回显复合尾参（此前尾参完全不回显）。
- **`crates/cli/src/run.rs::print_prompt_header`**：bare 命令不打印 header，compound 只打印尾参。
- **web SPA 零改动**：本地回显由 SSE `queue_consumed`/`steer_consumed` 驱动，`reduce.js` 对空 text 已判空跳过——发射源归一化后自然满足契约；持久回显（`/messages`）本就只有尾参。
- **存量 clippy 修复**（上一轮 working-tree 遗留，阻塞回归 Gate）：`shellguard/cd_tests.rs` 的 `match+panic!` 改单条 `assert!(matches!(...))`（`clippy::panic` 全仓 deny）。

## 同批落库项（同一 working-tree 收敛提交）

本 commit 与上述回显契约同批落库的其余主题（摘要级记录）：

- **skill 上下文收敛**（`crates/session/src/skill_context.rs`、`skill_lifecycle.rs`、`runner/llm_call.rs`）：skill body 改为逐调用 `[skill loaded]` 合成消息内联（模型不再烧一次工具调用读 SKILL.md），tail reminder 降级为空 body 兜底；run-end 清除收敛保证后续 run 的 payload 不再携带 body。测试：`skill_body_injection.rs`、`skill_tail_cleared_after_run_end.rs`、`skill_one_shot.rs`、`skill_context_tail.rs`。
- **build 子 agent 隐藏单一真源**（`crates/core/src/agent.rs::build_delegation_hidden`）：plan 模式恒隐藏 + 任意模式 task-plan 激活时隐藏，prompt 剥离与 tool schema 隐藏同源防漂移。测试：`agent.rs` 内 `build_delegation_hidden_matrix`、`crates/core/tests/skill_contract.rs`。
- **plan bash 拒绝话术收紧**（`crates/session/src/bash_guard.rs::plan_denial`）：context 收集显式路由到只读 `explore` 子 agent，而非换写路径重试 bash。测试：`bash_guard_plan_mode.rs`。
- **task-plan 高颗粒度 launch-closure 模式**（`crates/core/assets/skills/task-plan/SKILL.md` + `references/launch-closure-plan-checklist.md`）：五段结构 + 证据成熟度五级标注。
- **TUI**：vim bracketed-paste 字面插入（`vim/mod.rs::paste_terminal`，Command/Search 模式吞掉防泄漏）、镜像/回显/粘贴测试面扩充（`done_error_mirror_tests.rs`、`image_paste_tests.rs`、`steer_echo.rs`）。
- **触及区 memory**：`agents/session/index.md` 语义同步。

## 契约

**回显 ≡ 进 context**：用户在 transcript 上看到的回显文本 = `record_compound` 记录进 context 的文本。命令 token 两边都不出现；bare 命令两边都没有；`chat.rs` ctx 计量（按事件 `text` 估算）随之更准（bare 加 0、compound 只计尾参）。

## 功能 → 测试清单

| 功能 | 测试 | 位置 |
|---|---|---|
| consumed_echo_text 三态（compound 尾参/bare None/原文透传） | `consumed_echo_tails_compound_suppresses_bare_keeps_plain` | `crates/session/src/control_cmd.rs` |
| queue 复合消费只回显尾参、`/plan` 不外泄 | `queue_consumed_compound_carries_tail_text` | `crates/session/tests/queue_echo.rs` |
| queue bare 命令零回显、零 LLM、零记录 | `bare_control_command_queues_silently` | `crates/session/tests/queue_echo.rs` |
| steer 复合消费只回显尾参 | `steered_compound_plan_switches_then_runs_rest`（新增 SteerConsumed 断言） | `crates/session/tests/bare_steer_short_circuit.rs` |
| plain prompt 回显不受影响 | `queue_consumed_carries_text_and_precedes_output` | `crates/session/tests/queue_echo.rs` |
| SPA：尾参回显 / bare 无 user turn | `echoes consumed queue/steer prompts as user turns` | `crates/web/spa/src/reduce.test.js` |
| SPA：远端分发乐观回显走同一契约（bare 不渲染、compound 只渲染尾参） | `consumedEchoText tails compound suppresses bare keeps plain`、`remote optimistic echo never renders a bare control command` | `crates/web/spa/src/reduce.js`（`consumedEchoText`）+ `reduce.test.js`；`chat.jsx sendRemote` 接入 |

- 本轮验证（隔离 `CARGO_TARGET_DIR`，`--no-fail-fast -p session -p tui -p cli -p shellguard`）：
  108 套件 / 2837 passed / 4 failed。4 个失败**全部来自并行迭代的在途半成品**
  （`tools/latent.rs` question 解锁窗口、`no_session_row_side_effects.rs` 与
  `plain_skill_prompt.rs` 的 skill-activation 断言、一个 PoisonError 级联），
  均不在本轮触碰的回显链路上；本轮直接覆盖回显契约的套件全部通过
  （control_cmd 单测、queue_echo、bare_steer_short_circuit、compound_cmd、
  drain_mode、steer_followup、reabsorb_checks_queues、tui lib 1511、cli、shellguard）。
- clippy gate（`--workspace --all-targets -D warnings`）已转绿：唯一的
  `skill_context.rs:512` useless_format 属并行迭代文件，本轮代为做了单 hunk
  机械修复（`format!` 静态串 → 字面量），不改变断言语义。
- SPA 全套件 7 文件 / 58 tests 全绿（含 sendRemote 契约新测试）。
- 行数：迭代文件最大 `crates/tui/src/app.rs` 799 ≤ 800；无新增源文件入库

## 提交切分预案（02:36 法证修订版；B sidecar 批次落地后执行）

B 以 scoped pathspec 分批提交（已落 `ae7cb84`/`4337e4f`/`ff4a90c`/`c0cbece`/
`92c5e40`/`08a21ac`/`a16c0ba` 七个），不吞并 A 的 staged 内容
（`git log -S consumed_echo_text` 为空）。据此对旧预案做三处修正：

- **三个"混合文件"退出 A 清单**：`event.rs` 的 Queue/SteerConsumed doc 行与
  `chat.rs` 的 SteerConsumed 归一化 hunk 均已位于 staged 侧，将被 B 的 sidecar
  批次顺带扫走（doc/展示侧，零行为风险，B commit 号落地后回填引用）；
  `skill_context.rs` 的 clippy hunk 位于 B 新写的 truncation 测试区，语义归属
  B，不再由 A 代修提交。
- **`session/lib.rs` 简化**：B 落库其 staged 内容后，工作树仅剩 A 的
  `consumed_echo_text` re-export 单 hunk（unstaged），整文件 `git add` 即可。
- **A 最终闭包**：`control_cmd.rs`、`runner/{drain,steer}.rs`、
  `session/lib.rs`（B 落库后）、`cli/run.rs`、
  `spa/{reduce.js,reduce.test.js,chat.jsx}`、
  `tests/{queue_echo,bare_steer_short_circuit,compound_cmd}.rs`、
  `tui/src/chat_tests/steer_echo.rs`、本 changelog 与
  `2026-08-06/queue-steer-consumed-echo-text.md`；tui 侧
  `app_loop/app_helpers/app_loop_actions/app_submit` 归属在 sidecar 落库后复核
  再定（B 亦在编辑，staged/unstaged 分布未冻结）。

执行触发：`git log -S consumed_echo_text` 非空，或 `sidecar` 相关文件全部落库。
执行后门禁：A commit 独立 build + 回显契约 13 套件，随后干净树全量
`cargo test --workspace` + `clippy --workspace --all-targets -D warnings`。

## 落库门禁（隔离提交树实测，03:xx 更新）

本提交以 temp-index 自混合工作树分离落库（父 = 5ed895a，B 的 scoped 批次已
先行 push）。实测读数：

- `cargo build --workspace`：干净（隔离 CARGO_TARGET_DIR）
- session lib：**421 passed / 0 failed**（含 latent 解锁窗测试——其曾在
  a16c0ba 处于红，B 以 `b243c2b` 修复，本提交继承修复）
- 契约集成：queue_echo 3 / bare_steer_short_circuit 2 / compound_cmd 5 全绿
- cli + session 合计 scoped 读数 920 passed / 1 failed（唯一失败即上述
  latent 在途项，变基后消失）
- clippy：`-p opencoder-session -p opencoder-cli --all-targets -D warnings`
  零警告
- SPA vitest 全套件 **7 文件 / 58 tests 全绿**（spa 三文件与提交内容一致）
- 已知余项：全 workspace test/clippy 门禁待并行迭代 B 收敛后重跑（其
  sidecar 批次尚有 9 文件在途；触发与步骤见上节预案）
- 顺带落库说明：`event.rs` 字段 doc、`tui/chat.rs` SteerConsumed 归一化、
  `skill_context.rs` clippy 修复在 B 的 index 中随其 sidecar 批次落库，
  届时回填其 commit 号

## Related Docs

- [queue-steer consumed echo text](../2026-08-06/queue-steer-consumed-echo-text.md)（本轮修改其语义：事件 text 从 raw 全文收敛为 model-facing 回显）
- [queue 消费镜像](../2026-07-07/queue-consume-mirror-and-integrity.md)（QueueConsumed 事件语义演进：seq → seq+raw text → seq+model-facing echo）
