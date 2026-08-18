Commit: (working-tree, post-7cf8dfb)

# 排队 `$skill` 改为消费时刻激活（P0：立即触发缺陷）

## Context

TUI 三个提交口（`app.rs` Submit / Steer、`queue_admitter.rs` Tab-Queue）在**提交时**就调
`resolve_persist`：解析 `$name` → 写入与 runner 共享的 `SessionState.skill_prompt` Arc →
落盘 `sessions.skill`。而在跑的 turn 每次 LLM 调用都实时读该 Arc
（`llm_call.rs` → `skill_context::tail_reminder` + `latent::unlocked_from_body`），于是
`$skill` 刚入队、下一个 LLM 请求就带 `[active skill]` 提醒并解锁 latent 工具——skill
“立刻触发”，穿透了排队语义。session crate 侧本就有正确的消费时解析
（`drain.rs`/`steer.rs` → `record_compound`），TUI 的急切解析是越权的重复，且把 queue 行
prompt 剥成无 token 文本，消费方失去再解析的输入。

## Change Summary

- **session（激活时机唯一权威 = 消费时）**
  - `skill_resolve.rs`：新增 `persist_active_skill(session, prev)`（best-effort
    `update_session(skill=…)`，镜像 TUI `persist_skill`；只写不清，清除仍归 control_cmd /
    plan handoff / TUI `$` 菜单）；`record_compound` 解析前后捕获/落盘，保证消费后 resume
    不丢；`resolve_inline_skills_with` 补 `set_active_skill_names`（与 `cli/run.rs` 对齐）。
  - `runner/mod.rs` 直发 prompt 路径：`resolve_inline_skills` 后同样落盘（headless
    `$skill` 此前完全不持久化，顺带修复）。
- **TUI 去掉 queue/steer 路径的急切激活**
  - Submit 臂：`running` 时走 `handle_queue` 原文入队（含 `$name`），不调
    `resolve_persist`；删除“pure-skill 运行中→队列 trigger”分支（原文 `$name` 入队，
    `record_compound` 注入 `SKILL_TRIGGER`）。idle 路径不变（turn 立即开始，急切激活正确）。
  - Steer 臂：原文入 steer（复合 `/plan …` 的 `pending_plan_arm` 武装保留，按原文判定）；
    删除 pure-skill steer trigger 分支。
  - `queue_admitter.rs::handle_queue`：签名收缩（去掉 skill 相关 7 参，改同步 fn）——
    原文（token 保留）入队 + display=原文；`pending_plan_arm` 武装移入。
  - 死代码清除：`skill_display.rs` 的 `queued_item_display`/`skill_token_display`
    （display/prompt 分离的存在理由消失）及各自测试、app.rs 重导出。
- **TUI 镜像同步**：消费后 runner 更新共享 Arc，TUI 本地 `active_skill/_body` 会陈旧；
  `TurnDone` 后（idle 时）经 `app_helpers::refresh_skill_mirrors` 从 handle 刷新，name 由
  `> Source: /skills/<name>/SKILL.md` 前缀派生（`skill_display::skill_name_from_body`，
  多 skill 取首块名）。
- **文档/记忆**：`skill_persist.rs` 模块注释改“idle-Submit 专用 + 时机契约”；
  `store/tests/display_text.rs` 前提更新（queue 行 prompt 可含 token；给 LLM 的记录消息
  永不含）；`agents/tui/index.md` repair-on-touch（control_helpers 收窄为 idle、
  queue_admitter 补延迟激活语义）。

## 行为变化（有意）

- queue/steer 行的 `prompt` 与队列面板/`QueueConsumed` echo 现在显示原文（含 `$name`），
  更贴合用户所见；消费方只有 runner `record_compound`（剥 token 后记录）。
- queue 路径失去提交时“未解析 token”即时警告：runner 原样保留 `$bogus` 文本并回显。
- web `POST /prompt` 的显式 `skill` 字段是会话级 sticky 语义（等价 `$` 菜单），不在本
  P0 范围，维持现状。
- steer 路径同样消费时激活：mid-turn admit 的 `$skill` steer 不影响已发出的当 turn
  请求，边界吸收后才激活（`steer_skill_deferral.rs` 钉死该契约）。

## Validation（当次实跑）

- `cargo test --workspace`：**2921 passed / 0 failed**（exit 0）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `cargo build --workspace`：Finished dev profile。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `tui/tests/queued_skill_drain.rs::queued_skill_fires_at_consumption_not_during_kickoff` | e2e（重写） | **核心 P0**：kickoff 运行中排入 `$haiku fix the bug` 后，kickoff 请求任何消息不含 `[active skill]` 且 system 无 skill 正文；drain 后的 turn 末条 user 为 tail reminder（含 `haiku/SKILL.md`）；记录文本含 "fix the bug" 且永不含 `$haiku`；`sessions.skill` 入队时为 NULL、消费后落盘（`> Source:` 前缀）|
| `session/tests/steer_skill_deferral.rs::steer_admitted_mid_turn_defers_skill_until_absorption` | integration（新增，steer 路径核心） | **steer 延迟语义**：turn 1 停在 gate tool 执行中（请求已发出）时 admit `$review` steer——请求 #1 全 payload 无 `[active skill]` 亦无 `$review`、`sessions.skill` 仍 NULL、pending 行保留原文 token；开闸后边界吸收：请求 #2 末条 user 为 tail reminder（`skills/review/SKILL.md`）、steer 文本剥 token 记录、消费时落盘（`> Source:` 前缀）|
| `session/tests/plain_skill_prompt.rs::queue_pure_skill_prompt_injects_trigger` | integration | pure `$review` 单独入队 → 消费时激活 + 注入 `SKILL_TRIGGER`（synthetic），token 不入 transcript |
| `session/tests/plain_skill_prompt.rs::queued_skill_persists_at_consumption_not_admit` | integration | 落盘时机：admit 后仍 NULL，run 消费后 = 内存 body（verbatim） |
| `session/src/skill_resolve.rs::record_compound_persists_resolved_skill_to_store` | unit | `record_compound` 持 store 会话：解析即落盘，带 `> Source:` 前缀 |
| `tui/src/queue_admitter.rs::handle_queue_admits_raw_text_and_defers_skill` | unit | running 提交原文入队（prompt=display=原文含 token），queue 面板镜像含 token，`sessions.skill` 不动 |
| `tui/src/queue_admitter.rs::handle_queue_pure_skill_admits_token_not_trigger` | unit | pure-skill 入队的是 `$alpha` 原文而非合成 trigger 文本 |
| `tui/src/app_helpers_tests/skill_apply.rs::refresh_skill_mirrors_syncs_name_body_and_tokens_from_handle` | unit | 镜像同步：name 派生/body/`sys_tokens` 重估；无漂移 no-op；清除路径清镜像 |
| `tui/src/skill_display.rs::{name_derived_from_source_prefix, multi_skill_body_uses_first_block, body_without_source_prefix_has_no_name}` | unit | name 派生前缀格式、多 skill 首块、无前缀 → None |
| `tui/src/skill_persist.rs::persist_skill_survives_combined_idle_skill_submission` | unit（前提收窄） | 原“queue 时落盘”用例收窄为 idle 提交路径（queue/steer 已改延迟） |

删除：`skill_display.rs::queued_item_display`×4、`skill_token_display`（含
`app_tests/skill_tests.rs` 1 例）——被剥 token 的 prompt/display 分离契约随急切解析一并
消失，其 store 层不变量仍由 `store/tests/display_text.rs` 全量覆盖。
