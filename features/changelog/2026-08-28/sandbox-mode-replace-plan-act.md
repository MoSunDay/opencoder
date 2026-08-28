Commit: (working-tree, sandbox 模式替换 plan/act 双模式)

# sandbox 模式替换 plan/act 双模式

## 背景

plan/act 双模式职责混乱：plan agent 既承担只读探索又承担「产出结构化计划」（Goal/TODO/Verify/Risks/Align 模板），并以 `plan_snapshot`/`plan_input_count` 持久化相位状态（arming 计数器、快照救援、resume backfill）支撑 Shift+Tab 手动切换，机制面大、语义面窄。本轮把「规划」降级为一种 skill 能力（`task-plan`），把「只读」升格为独立的 **sandbox** agent，模式系统收敛为两个纯状态切换命令 + 一个折叠执行命令。

## 实现

- **core**：`AgentKind::Plan` → `AgentKind::Sandbox`；内置 agent `plan` → `sandbox`（Primary，bash/task/question），`act` 的 ToolFilter 增 `question`（可见性由 session 门控）；`base_prompt_plan`/`PLAN_SUFFIX` → `base_prompt_sandbox`：只保留只读约束 + question 澄清指引，删除「必须输出计划/模板」语义。`KeymapConfig` 16 键——删除 `switch_mode`(ctrl+t)/`switch_mode_clear`(alt+tab)/`switch_mode_keep`(ctrl+shift+tab) 三绑定；旧用户 JSON 中这些键被 serde 容忍（有回归测试）。
- **session**：
  - 控制命令（`control_cmd.rs`）：`/sandbox`=SwitchAgent("sandbox")、`/act`=SwitchAgent("act")——纯状态切换 + 持久化 + `AgentSwitch` 事件，**不做任何折叠**；`/act_clear_context` 为 canonical ClearContext（`/clear_context` 为接受别名）。`plan_phase.rs` 整文件删除（arming 计数器、快照、tag/persist/reset 全退场）；`plan_handoff.rs` → `handoff.rs`，折叠门改为纯 provenance——**存在最新非空 assistant 文本才折叠**为 continuity seed 并执行，否则空白 fresh-start sentinel。
  - 只读机器不换骨：bash 写拦截文案改 "Blocked in sandbox mode"；sandbox 下仅 `explore` 子代理；task schema 隐去 build；prompt 注入 `IN_SANDBOX_MODE`。
  - **question 工具转 latent**（`tools/latent.rs`）：`task-plan`/`review` skill 解锁；可见性谓词 `latent::is_visible` 由 `llm_call.rs` 与 `estimate_tool_schema_tokens` 共用（token 估算不漂移）——**sandbox 恒可见，act 需 skill 解锁**（无 skill 的 act 看不到 question）。seed 资产 `task-plan/SKILL.md`、`review/SKILL.md` 把 question 澄清协议前移到正文前 500 字符解锁窗口内。
  - autopilot：`ap` 规划阶段 = sandbox agent + 激活 **task-plan** skill + continuation prompt（原 review skill）；`review` pass 去掉 switch-to-plan 与 act-only gate——留在当前 agent + review skill，任意模式可跑。
- **store/node**：`SessionMeta`/`SessionPatch` 删除 `plan_snapshot`/`plan_input_count`（**schema v10 列保留不迁移**，运行时不读写）；读路径单点 `normalize_agent` 把存量 `agent="plan"` 归一为 `"act"`（原始行不改写）。`handoff_seq`/`handoff_plan` 机制保留。
- **cli**：`--agent` clap 解析期校验（`plan` 报错并提示 renamed to sandbox）；`rewrite_legacy_plan_prefix` 把前导 `/plan …` 重写为 `/sandbox …`；`display.rs` 新增 `TranscriptReset` 折叠横幅；`session show --json` 无 plan 专属字段。
- **tui**：命令面板 `/plan`→`/sandbox`（只读沙箱）、`/act`（退出沙箱）；Shift+Tab 重绑为派发 `/act_clear_context`（与键入等价的纯控制命令路径，`SwitchAndStart`/`gate_switch`/`dedup_switch`/`mode_switch.rs` 全退场，agent 切换统一走 runner、chip 由 `AgentSwitch` 事件驱动）；左下 chip `[sandbox]`（warn 橙）。plan 卡片（`ChatBlock::Plan`）、Shift+I plan 编辑器、`/annotation` 保留。
- **web**：`post_agent` 只接受 primary `act|sandbox`（其余 400）；`DrainCmd::ResetPlanPhase` 删除；`POST /handoff` 保留（语义=折叠最新 assistant brief 执行）；SPA `reduce.js` 对 `agent_switched` 值泛化（新增 vitest 断言 sandbox 渲染 + 未知值不崩）。

## 行为变化

- `/sandbox` ⇄ `/act` 纯状态切换：不折叠、不耗 LLM turn、持久化、`/act` 在 act 下 no-op。
- 规划需求改由 `$task-plan` 承担：解锁后 act 可用 question 主动澄清；sandbox 恒可提问（沿用旧 plan 行为）。
- `agent_switched` 事件值域出现 `"sandbox"`；`PlanHandoff` 事件已不存在（折叠边界统一走 `transcript_reset`）。
- 旧库存量 `agent="plan"` 会话 resume 归一为 act；`--agent plan`、`/plan` 提交有明确迁移路径（报错提示 / 前缀重写）。

## 测试清单（rules/02 全量回归）

`cargo test --workspace`：**232 套件 / 3308 通过 / 0 失败**。要点锚点：

- session：`bash_guard_sandbox_mode`（rm 被拦/ls 放行/文案）、`sandbox_subagent_guard`（build 拒绝、仅 explore）、`agent_switch_roundtrip`（纯切换不折叠、/act no-op、持久化）、`question_gating`（act 无 skill 不可见、task-plan/review 解锁、sandbox 恒可见）、`legacy_plan_agent_resume`（plan 行归一 act）、`latent_tools`、`control_cmd`（idle 短路/队列 [/sandbox,prompt,/act] FIFO）、`clear_context_toggle_regression`（seed/sentinel 分叉）、`handoff_*`（折叠门 provenance）、`agent_model_toctou`；lib 内 `sandbox_mode_task_schema_omits_build`、estimator 镜像、latent 15 例。
- tui：`act_clear_context_fold`（折叠为单条 seed、1 turn 执行、resume 重建、复合 rest、空白 sentinel）、`handoff_provenance_gate`（纯最新 assistant 文本门）、`agent_switch_persist`、`switch_blocked_while_running`（MockChatClient FIFO）、`plan_card_dedup`、status_bar chip 渲染（warn 色）、`backtab_and_typed_clear_context_are_one_path`（Shift+Tab ≡ `/act_clear_context`）。
- store：`legacy_agent_normalization`（raw UPDATE 'plan' → 读回 'act'，原始行不动；'sandbox' 直通）；`store_migrations` 保留列存在断言；`plan_phase.rs` 删除。
- web：`running_mode_gate`（post_agent 收 sandbox / 拒 plan 400 且零足迹；`agent_switched` 值为 "sandbox"）、`replay_fidelity`（事件面全枚举 21 变体防漂移）、`web_contract`/`web_api_ops`/`web_drain_cmds`；spa vitest 28/28（reduce 15 + sign 13）。
- cli：`headless_agent_compound`（legacy `/plan` 前缀重写进 runner、bare `/plan` 零 LLM turn、`/sandbox $skill text` 复合）、`cli_parse`（--agent 拒 plan 带提示/收全量 primary）。
- 根 e2e：`running_mode_switch_e2e`（真二进制：running 中 `/agent`/`/handoff` 409、`agent` 字段 409、文本命令排队 idle 边界生效，plan→sandbox 语义）。
- core：`skill_contract` 23 例（task-plan/review seed 前 500 字符 question 指引 + body 窗口解锁）、keymap 16 键 + legacy JSON 容忍、agent 注册表/prompt 结构测试重写。

## 边界

- `seed_builtin_skills` 不 clobber：旧机器上已 seed 的 `task-plan`/`review/SKILL.md` 不会自动获得前移后的 question 指引（解锁靠前 500 字符匹配，旧 seed 若正文含技能名仍可命中；必要时删 `~/.opencoder/skills` 对应目录重新 seed）。
- question 转 latent 是行为收窄：headless（无 hub attach）仍走 `NO_LISTENER_REPLY` 兜底，但无 skill 的 act 会话不再主动提问——按需求即为预期。
- schema v10 的 `plan_snapshot`/`plan_input_count` 列保留（兼容旧库，无破坏性迁移）；`plan_handoff` 命名仅存于历史 changelog 链接。
