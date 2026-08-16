# Skill context 移出系统提示：逐调用尾部瞬时提醒（prefix-cache 稳定）

## Context

skill 正文此前经 `build_system` 的 `skill_prompt` 参数以 `## Active skill` 段追加到系统提示末尾：每次激活/替换 skill（以及任何未来的默认注入开关变化）都会改写 payload 的**首字节**，直接击穿 provider 的 prompt-prefix cache；同时正文 tokens 每轮全量预付，即使任务并不需要。新模型之前，TUI 运行中经共享 `Arc<Mutex>` 置 skill 的修复（见 `skill_mid_run.rs`）已让 skill 能在 turn 间生效，但注入位置本身仍是系统提示。

新模型：**每次 LLM 调用**从 session 状态派生一条合成 user 消息追加在 payload **末尾**（`[skills]` 目录 + `[active skill]` 源路径），不落库、不回放、压缩后按需重生成——系统提示恢复字节稳定，模型按需懒读 SKILL.md。

## Change Summary

- 新增 `crates/session/src/skill_context.rs`（202 行）：`source_path_from_body`（解析 `> Source: <path>` 前缀，`opencoder_core::body_with_source` 写入）、`catalog_entries(_in)`（`enabled_skill_names()` ∩ `discover_in(root)`，按名排序）、`reminder_text`（纯文本拼装 `[skills]`/`[active skill]` 段 + 懒加载指引）、`tail_reminder(session)`（仅 Primary 且非 `workflow`；产出 `synthetic=true` 的 user 消息）。
- `crates/session/src/runner/llm_call.rs`：`build_system` 调用去掉 skill 参数，`to_send` 末尾追加 `tail_reminder`（派生于当次调用，永不持久化）；`crates/session/src/prompt.rs`（-11 行）删除 `## Active skill` 段——`build_system` 与 skill 彻底解耦，system 字节稳定。
- `crates/session/src/compaction/mod.rs`（+15 行）：token 估算补计尾部提醒（提醒本身不落库，压缩摘要后下一调用自动重生成，无需特殊处理）。
- `crates/core`：新增 `config/skill.rs`（23 行，`SkillConfig { enabled }` + 条目级 merge）与 `Config.skills: HashMap<String, SkillConfig>`、`enabled_skill_names()`（按名排序）；`config.rs` +19 / `config/merge.rs` +15 / `config/tests.rs` +61（4 个新测试）。
- TUI `/skill` 弹窗（与 `/cli`//`/mcp` 对齐）：新增 `crates/tui/src/skill_menu/`（`mod.rs` 11 + `list.rs` 150 + `state.rs` 37 + `view.rs` 97 行；发现结果合并 config 开关、缺省 OFF、`←/→` 翻转即存、Enter/Esc 关闭、Ctrl-D 兜底）与 `app_loop_skill.rs`（47 行：JSON merge-patch 保存 → reload → `UiCmd::ReloadConfig` → `[/skill] saved → <path>` 标记，失败红字）；`command.rs` `SlashAction::Skill`（别名 `/sk`，+12 行）；`app.rs`/`app_loop.rs`/`app_loop_actions.rs`/`render.rs`/`frame.rs` 接线（弹窗打开时吞键、composer 上方弹层）。
- autopilot 适配：`crates/session/src/autopilot/phases.rs` review skill 激活改存 `body_with_source(&Skill)`（带 `> Source:` 前缀，尾部路径提醒可解析）；`crates/tui/src/app_helpers.rs`（+15 行）submit 前置 token 估算改用无 skill 的 `build_system` + 显式补计尾部提醒。
- 测试迁移与新增：`crates/session/tests/skill_mid_run.rs`（+118 行，断言从「system 段」迁移到「尾部提醒」）、`tests/prompt.rs`、`crates/tui/tests/queued_skill_drain.rs`、`crates/tui/tests/plan_act_handoff.rs`、`crates/tui/src/app_tests/skill_tests.rs`；新增 `crates/tui/src/app_loop_slash_action_tests.rs::slash_action_skill_parses_and_opens_toggle_menu`（+67 行）；新增集成 `crates/session/tests/skill_context_tail.rs`（383 行，5 个测试，`HOME_LOCK` 串行 + 真实 skill 文件的 tempdir HOME）。

**明确不动**：`$` picker 激活语义与 `skill_resolve` 的 `$name` token 剥离；latent 工具解锁（`unlocked_from_body` 仍读正文全文）；skill 的 store 持久化/resume（`skill_resume.rs`）；压缩摘要内容。

## Validation

- `cargo test --workspace --no-fail-fast` → 全绿（164 个测试二进制，**2670 passed / 0 failed**；基线 2652 → +18：core config ×4、session skill_context 内联 ×4、集成 skill_context_tail ×5、TUI skill_menu ×5 + slash-action ×1，prompt.rs 2→1 净 -1）
- `cargo clippy --workspace --all-targets -- -D warnings` → Finished，零警告
- `cargo build --workspace` → Finished，零错误

### 测试覆盖

| 测试 | 断言 |
| --- | --- |
| core `config::tests::skills_default_empty_and_enabled_names_follow_toggles` | `{}` → 空 skills/空启用表；缺 `enabled` 反序列化为 false；`enabled_skill_names()` 只含启用项 |
| core `config::tests::merge_into_skills_preserves_sibling_entries` | JSON merge-patch 按条目合并，兄弟开关保留 |
| core `config::tests::enabled_skill_names_are_sorted` | 启用名按字典序稳定排序 |
| core `config::tests::load_reads_skills_from_config_file` | `opencoder.json` 的 `skills` 段正确载入（HOME 隔离） |
| session `skill_context::tests::source_path_from_body_variants` | `> Source:` 前缀解析/缺失/空路径边界 |
| session `skill_context::tests::catalog_entries_in_intersects_and_sorts` | 启用名 ∩ 发现结果、按名排序、未启用项剔除 |
| session `skill_context::tests::reminder_text_sections` | 纯拼装：`[skills]` 头 + 条目 + 懒加载指引、段间空行、`[active skill]`、空输入空串 |
| session `skill_context::tests::tail_reminder_gating_and_content` | 无内容 → None；Primary + Source 前缀 → synthetic user 消息；workflow/explore 排除 |
| 集成 `skill_context_tail::system_prompt_bytes_stable_across_catalog_and_activation_changes` | 目录开关 + 激活 skill 中途翻转/回退，system 消息**三方字节级相同**，尾消息按预期出现/消失 |
| 集成 `skill_context_tail::skills_catalog_reminder_is_last_payload_message_and_never_persisted` | 目录提醒是 payload 最后一条（真实用户文本不再居尾）、含目录路径/`- alpha: …`/懒加载指引、禁用项不在目录、永不落入 `session.messages` |
| 集成 `skill_context_tail::active_skill_source_path_rides_tail_reminder_and_keeps_system_clean` | `> Source:` 正文只出路径：system 与提醒均不含正文文本 |
| 集成 `skill_context_tail::legacy_body_without_source_prefix_yields_no_active_skill_section` | 无前缀旧格式 body → 全 payload 无 `[active skill]`（解析契约） |
| 集成 `skill_context_tail::subagent_and_workflow_payloads_carry_no_skill_context` | explore/workflow 即便有启用目录 + 激活 skill，payload 全程无 `[skills]`/`[active skill]` |
| 集成 `skill_mid_run::skill_set_mid_run_appears_in_next_turn_tail_reminder` | 运行中经 `Arc<Mutex>` 置 skill：下一 turn 尾提醒携带源路径、system 干净、当 turn 无痕迹 |
| 集成 `skill_mid_run::skill_set_mid_run_appears_in_queue_followup_turn` | 队列 follow-up turn 同样携带尾提醒 |
| 集成 `skill_mid_run::skill_only_empty_prompt_starts_turn_with_skill_tail_reminder` | 空 prompt + skill → 恰一次 LLM 调用且尾提醒在场 |
| 集成 `skill_mid_run::skill_only_empty_prompt_records_user_trigger_message`、`image_only_turn_with_skill_records_both_user_image_and_trigger` | 合成触发消息（及图片消息）正确落 transcript |
| 集成 `skill_mid_run::set_skill_and_clone_roundtrip`、`with_skill_builder_sets_skill` | setter/builder 与共享 `Arc` 一致 |
| session `tests/prompt.rs::build_system_contains_no_skill_section` | `build_system` 输出与 skill 彻底无关 |
| TUI 集成 `queued_skill_drain::queued_combined_submission_drains_with_skill` | 队列组合提交在 idle 边界 drain 出 skill turn |
| TUI 集成 `plan_act_handoff::switch_and_start_clears_skill_prompt` | plan→act 切换启动清空 skill_prompt |
| TUI `command::tests::parse_known_commands`、`short_key_command_removed` | `/skill` parse/dispatch、`/sk` 别名改指 `/skill` |
| TUI `skill_menu::list::tests::from_discovered_merges_config_and_defaults_off` | 发现列表合并 config 开关、缺省 OFF |
| TUI `skill_menu::list::tests::toggle_json_shape_on_and_off`、`left_arrow_toggles_selected_and_stays_open`、`move_up_down_wrap`、`enter_esc_close_and_empty_list_keys_are_noops` | 翻转产 `{"skills":{<name>:{"enabled":…}}}` 且弹窗保持、导航/关闭/空列表键位 |
| TUI `slash_action_skill_parses_and_opens_toggle_menu` | `/skill` 与 `/sk` 打开 `SkillMenu::List` 模态（真 discovered 列表） |

## Compatibility

- 系统提示字节稳定 → provider prefix cache 命中率恢复；`## Active skill` 段**已删除**（旧 transcript 不受影响：resume 只回放消息，system 每次重建）。
- 风险：每次调用多付目录 + 路径提醒的少量 tokens（固定、小）；模型匹配到 skill 时需**多读一次** SKILL.md（懒加载换预付正文）；无 `> Source:` 前缀的旧格式 body 不产生 `[active skill]` 段（正文仍解锁 latent 工具，但不会被提醒指路）。
- `/skill`（`/sk`）保存即写 config（JSON merge-patch、项目优先），与 `/cli`//`/mcp` 行为一致；开关只在下一模型调用生效。
