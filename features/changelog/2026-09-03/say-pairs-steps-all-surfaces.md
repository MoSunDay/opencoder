Commit: 2677992

# N Steps 与 Say 成对：SPA 双路径 + TUI replay 补齐 Say 收合契约

## Context

Turn 契约 `1 Turn = n Steps + Say`（a02eea1 在 TUI live 落地）要求：一次用户输入可含**多个**配对——每个非空 Say 关闭当前子 Turn，其后的 reasoning/tool 落在 Say **之下**开新阶梯，新 Say 出来之前的 step 归拢在同一阶梯里。三处违背：

1. **SPA live**（`steps/reducer.js`）：`turnStepsIndex` 从 user 边界向后找**第一个** steps turn——Say 之后的回合并回 Say 之上的旧阶梯，回合 2 的 step 显示在中间 Say 上方。
2. **SPA snapshot**（`reduce.js::turnsFromMessages`）：同一缺陷；`toolAhead` lookahead 让 Say 前的 pending thinking 跨过 Say 等待未来 tool。
3. **TUI replay**（`normalize_turn_groups`）：只按 User 分段、段内全部阶梯合并到首个 Say 之前——resume 后 `[组, Say, 组, Say]` 塌成 `[合并组, Say, Say]`，与 live 相反；且 `flush_segment` 产出 `thinking: Vec::new()`，replay 的 step 永不携带已渲染 thinking（a02eea1 当轮跳过测试，遗留 13 例失败）。

## Change Summary

- **SPA `steps/reducer.js`**：锚点改 Say 感知——`turnFloor` 取最后一个非空 assistant Say 之下（`image:true` 标记与空文本不算 Say）；`turnStepsIndex`/`turnInsertIndex`/`turnHasSay`/`settleTurnProgress` 全部以 floor 为准（settle 只冻结正被新 Say 收合的那条阶梯，其下新阶梯动效重新点亮）；`placeThinkingStep` 重写为 TUI `place_thinking_step` 镜像（Say 透明、阶梯吸收 call-less step、其它块封顶）。
- **SPA `reduce.js::turnsFromMessages`**：改为 TUI replay 的 BLOCK ORDER 语义——非空 assistant Text 恒为收合点：先把缓冲回合（含悬空 reasoning）`appendStepTurn` 进 Say 之上的阶梯再推 Say；删除 `stepToolAt`/`toolAhead` lookahead；空 assistant 文本不渲染不收合；image 标记 `image:true` 渲染但永不收合。
- **SPA `bubbleItems.js`**：run 在自己的 Say 处**含尾**截断——每个 `assistantTurn` 气泡 = 一条合并阶梯 + 它的 Say；Say 之后的 steps 开下一个 item；run 内 sys 状态行被吸收进尾部的 sys 渲染（`transcript.jsx::SysContent`），不再拆散配对。
- **SPA `chat.jsx`**：`startStream` 从 `initialTurns` 末尾的 user 文本播种 `pendingEcho`（TUI `push_user` + `pending_turn_echo` 镜像，裸控制命令无回显则置 null）；`transcript_reset`/done 快照重载经 `reduce.js::ensurePendingEcho` 重推 store 尚未落库的 User 边界（`/act_clear_context <tail>` 对齐）。
- **TUI `chat_steps.rs::normalize_turn_groups`**：分段点 User 之外增加 Assistant——Say 封顶子段，段内相邻组合并、跨 Say 不再上翻；`session_ui/replay.rs::flush_segment`：`thinking: markdown::render(&thinking_raw)` 立即渲染（live 在 absorb 时即渲染）；`chat_stream.rs` 模块注释按配对契约重写（旧"answer fragments never split"已过时）。
- **TUI `round_assistant_idx` 修复锚**（`chat_types.rs`/`chat_stream.rs`/`chat.rs`）：`reconcile_completed_assistant` 只修**本 run 打开的 Say**（Say push 时设锚、turn 准入清锚、锚不消费）——修掉背压丢帧时改写上一 Turn Say 的真 bug；本轮 `TextDelta` 全被丢弃时改为 INSERT 恢复的 Say 而非改写，重复 `AssistantFinal` 幂等重修。
- 语义边界（live/replay 双面镜像一致）：遗留态孤儿 thinking（Say 之后无新 Say/工具即 run 结束）折入 Say **上方**的旧阶梯而非悬空新梯——该路径仅历史状态可达，为避免无 Say 悬空梯而保留。
- 修复 a02eea1 遗留的 13 例测试（stale 期望，live 行为未动）；重建 `spa/dist`（check-spa-drift 通过）。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| TUI replay 按 BLOCK ORDER 重建配对：[User, 组(a), Say, 组(b), Say] | `chat_tests/step_group/replay.rs::replay_pairs_each_ladder_with_its_own_say`（新增） |
| TUI replay 阶梯跨 Say 不上翻、live/replay 同构 | 同文件 `live_and_replay_share_one_turn_with_the_same_two_steps`（修复） |
| TUI replay step 携带已渲染 thinking | `replay_absorbs_thinking_behind_assistant_text_like_the_live_path`、`replay_folds_thinking_without_a_following_group_into_the_ladder`、`session_ui/replay_duration_tests.rs::replayed_reasoning_folds_into_the_ladder_step`（修复/更名） |
| TUI live：Say 后 tool 开新梯、按 id 路由 | `chat_tests/tool_collapse/turn_routing.rs::text_between_calls_splits_its_turn_and_routes_by_id`（更名修复） |
| TUI live：背压丢帧不误改上一 Turn 的 Say；全丢帧 INSERT 恢复；重复 AssistantFinal 幂等 | `chat_tests/reconcile_repair.rs` ×4（新增：in-place 修复 / 全丢帧插入新 Say / 多 Say 修最后一个 / 重复终帧幂等） |
| SPA live：tool a→Say→tool b→Say 得两对，回合 2 动效重亮、只被自己的 Say 冻结 | `spa/src/steps/reducer.test.js` (d2)（新增）、(c2)/(d) 随新语义更新 |
| SPA snapshot：同流消息得 `[user, steps[a], say, steps[b], say]` | `spa/src/reduce.test.js::pairs Say-closed sub-turns…`（新增）+ 空 assistant 文本跳过（新增） |
| SPA 气泡：run 在 Say 含尾截断、两对两气泡；image/空文本不截断 | `spa/src/bubbleItems.test.js`（更新 + 两对拆分新增） |
| image 标记不是 Say（live 不收合） | `spa/src/steps/reducer.test.js` (k2)（新增） |
| SPA：/seq 快照先于 POST（丢帧竞态）；reset 重建重推 User 回显 | `spa/src/chat.dom.test.jsx`（新增 ×4） |

## 回归

- `cargo test -p opencoder-tui --lib`：1645/1645（基线 13 失败全修复 + reconcile_repair ×4）。
- `cargo test --workspace`：全绿（253 个 test result 块 / 3997 passed / 0 failed）。
- `cargo fmt --check` / `cargo clippy --workspace --all-targets`：0 warning（含 bash_guard.rs 遗留孤儿 doc 注释归位）。
- `cd crates/web/spa && npx vitest run`：169/169（15 文件）。
- `scripts/build-spa.sh` + `scripts/check-spa-drift.sh`：dist 已重建、no drift。

## Related Docs

- [Say 收合 Turn](say-closes-turn-transcript-reset-echo.md)（本篇补齐其 SPA/replay 两个缺口并修正该篇对 SPA 的错误断言）
- [每个用户输入一个阶梯](turn-boundary-per-user-input.md)
