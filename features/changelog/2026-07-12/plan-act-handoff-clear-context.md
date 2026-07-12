Commit: (working-tree, pre-initial-commit)

# plan→act 切换：清空 ctx、只保留最终计划并执行

## 背景

plan 模式产出的计划是 plan agent 的纯文本输出（无 plan 文件）。用户 Shift+Tab 切 act 时，worker 的 `SwitchAndStart` 切 agent 后用**空 prompt** 进 `run_loop`——act 继承**完整 plan 对话 transcript**（探索杂音、subagent 调用、澄清 Q&A），既浪费 token 又让 act 被规划期噪声干扰。

用户要求：切换时清空 ctx，只把最终计划交给 act，并立即开始执行；执行过程 jsonl 保留。

## 关键澄清：隔离边界

- act 真实发给 LLM 的 context = `session.messages`（`runner.rs:321`）。切换前含完整规划对话 → 这是要清的点。
- store 是 append-only（`append_message`，`lib.rs:136`），`load_messages`（`resume.rs:38`）重载全部历史。压缩（`compaction.rs:83`）已是「纯内存重置 + store 保留 + resume 重载」范式 —— 本次完全沿用。

## 变更

### 新增 `crates/session/src/plan_handoff.rs`
- `pub fn handoff(session: &mut SessionState) -> bool`：取最后一条非空 assistant 文本为「最终计划」，把 `session.messages` 重置为单条**合成 user 指令**（计划全文 + 执行前缀 `HANDOFF_PREFIX`）。无计划 → 返回 false（安全回退，caller 不动）。
- `pub fn final_plan_text(messages: &[Message]) -> Option<String>`：最新非空 assistant 消息（跳过空/纯工具 turn）。
- 与压缩同范式：**只改内存 `session.messages`，不碰 store**（jsonl/审计完整保留）。

### `crates/session/src/lib.rs`
- `pub mod plan_handoff;`

### `crates/tui/src/worker.rs`（`SwitchAndStart`）
- agent 切换后、`run_session` 前：`if plan_handoff::handoff(sess) { emit TranscriptReset(sess.messages.clone()) }`。
- `TranscriptReset` → `app.rs:635` `replay_into_chat` 重建 → **UI 同步清空，只显示计划**。
- `SwitchAndStart` 仅由 `app.rs:506` 的 plan→act-with-content 路径触发（`plan_to_act && !blocks.is_empty()`），故 handoff 作用域精确。

### 「最终计划」选取
取**最后一条非空 assistant 文本**（plan agent prompt 要求其以「clear, actionable plan as text」收尾）。多轮规划（探索→提问→计划）下，最新 assistant 即最终计划。计划跨多条 assistant 的情况当前不聚合——按需后续增强。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 有计划 → 压成单条合成 user 指令、含计划全文 + 执行指令、丢早期杂音 | `handoff_keeps_only_final_plan` | `crates/session/tests/plan_handoff.rs`（新增） |
| 无 assistant 计划 → no-op（返回 false、messages 不变） | `handoff_noop_without_plan` | 同上 |
| 跳过空 assistant turn、取非空计划 | `handoff_skips_empty_assistant_turns` | 同上 |
| **store 不动**：真 `LibsqlStore::open_memory()`，record 后 handoff，`load_messages` 计数/内容不变（jsonl 保留契约） | `handoff_does_not_touch_store` | 同上 |
| `final_plan_text` 取最新非空 assistant | `final_plan_text_picks_newest_nonempty_assistant` | 同上 |
| `final_plan_text` 无 assistant → None | `final_plan_text_none_when_no_assistant` | 同上 |
| **集成**：真 `process_cmd(SwitchAndStart("act"))` → 发 `AgentSwitch` + `TranscriptReset`（单条含计划）；act 的 LLM 请求**结构上**只有 1 条 user（handoff）、0 条 assistant（规划对话未泄漏） | `switch_and_start_clears_transcript_and_feeds_only_plan_to_act` | `crates/tui/tests/plan_act_handoff.rs`（新增） |
| **集成**：无计划时 `SwitchAndStart` 不发 `TranscriptReset`、原 transcript 不被 handoff 改动（安全回退） | `switch_and_start_without_plan_falls_back_gracefully` | 同上 |

- 全量回归：`cargo test --workspace` → **325 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`plan_handoff.rs`(src) 51 ≤ 400；`plan_handoff.rs`(test) ≤ 400；`plan_act_handoff.rs` ≤ 400

### 验证设计说明
- **结构断言优于子串**：集成测试对 act 的 LLM 请求按 `role` 字段计数（user==1、assistant==0）并直接读 `content`，规避 JSON 转义 `\n` 导致子串误判；比 `to_string().contains()` 更强。
- **真边界 Mock**：集成测试用 `MockChatClient` 录制 `requests()`，验证「act 实际收到什么」——这是用户可感知契约，而非纯函数自洽。
- **store 契约实测**：`handoff_does_not_touch_store` 用 `LibsqlStore::open_memory()` + `record` 持久化后断言 `load_messages` 不变，落实用户「执行过程 jsonl 保留」要求。

## Impact Surface

- **TUI 用户**：plan→act 切换后，act 只看到最终计划并立即执行；UI 同步清空为只含计划。store（`session show --json`）仍保留完整原始规划记录。
- **resume 行为**：与压缩一致——重载全部历史（handoff 边界不跨 reload 持久化）。这是既有 trade-off，非本次引入。
- **不影响** web / CLI 的 agent 切换路径（`SwitchAndStart` 仅 TUI）。web/CLI 后续可复用 `plan_handoff::handoff`。
- **不影响** compaction / steer / subagent / bash_guard。

## Notes / Compatibility
- 无 config 开关，直接作为 plan→act 默认行为。无计划文本时安全回退到原行为（不改动 transcript）。
- jsonl/store 完整保留——满足「执行过程 jsonl 保留」要求。

## Related Docs
- [agents/session](../../agents/session/index.md)（已同步：plan→act 流程、plan_handoff 抽象、测试锚点）
- [plan-act 手动切换（初版）](../2026-07-06/plan-act-handoff-compact.md)
