# subagent 块不再拆散 Step 阶梯与 Say 的配对

一句话：模型先派发 subagent（`task` 调用）再作答时，TUI 的转录块序是
`[StepGroup, Subagent, Assistant]`——`task` 不入阶梯，但它打开的 `Subagent`
块被压在该轮阶梯与收尾 Say 之间。阶梯/Say 的配对判定此前要求严格相邻，
于是 Say 开流时阶梯不退场（subagent 行上方仍挂着 `▸ N Steps`），seal 后也
不合并（下方重新起一个 `❯ Say:`），用户看到的就是「subagent 之后 Step 被
跳过、Say 重新起」。现在 `chat_steps.rs` 新增 `say_after_group` /
`group_before_say` 两个纯查找：前后行走时跳过 `Subagent` 块、遇到其他任何
块即停，`Subagent` 块对配对透明，其余隔断（用户回显、旧 Say、新阶梯、
Marker、Compaction）照旧拆对。渲染（`chat_flatten.rs`）与命中区记账
（`chat_headers.rs`）共用同一查找，不会再各算各的：Say 开流即阶梯退场、
seal 后合并为 `▸ Say(n steps): <preview>`，subagent 行按块序落在合并头行
（及其空行）之后、Say 正文之前。`last_block_ends_blank` 同步精确化：正文
整块隐藏的合并对，阶梯紧邻时以头行空行收尾（边界不叠第二个空行），隔着
subagent 行时以 subagent 行收尾（边界照常补空行）。SPA 同症同治：
`itemsFromTurns` 此前把 `subagent` turn 当作 run 的终点，`[steps, subagent,
say]` 被拆成「无 Say 的 N Steps 气泡 + subagent 气泡 + 裸文本气泡」；现在
开放 run 内的 `subagent` 块与 `sys` 行一样被吸收进 `say`（块序），
`AssistantTurnContent` 把位于首个 Say 文本之前的 subagent 块渲染在头行与
Say 正文之间，首个 Say 文本之后的部分位置不变。

沿革：2026-09-03 `say-closes-turn-transcript-reset-echo.md` 确立「1 Turn =
n Steps + Say」与 Say 关轮；本次只放宽配对的相邻要求，不改 Turn 边界、
不改 `turn_block_start` 推进规则（`SubagentStart` 本就不推进）。

改动面：`crates/tui/src/chat_steps.rs`（新增两个查找）、`chat_flatten.rs`
（Assistant 臂 `merged`、StepGroup 臂 `follower`、`last_block_ends_blank`）、
`chat_headers.rs`（`merged_say` 闭包、StepGroup 臂 `say_streams`、
Assistant 臂 `merged`）、测试镜像 `chat_tests/line_accounting.rs`、
新增 `chat_tests/subagent_steps_continue.rs`（`chat_tests/mod.rs` 注册）；SPA `crates/web/spa/src/
{bubbleItems.js,transcript.jsx}` 及 `bubbleItems.test.js`、
`stepsBlock.dom.test.jsx`。仅 TUI/SPA 转录展示与点击记账，不触及规则
04/06/07 契约，无公共能力/CLI/API/依赖变更，不触发 Ontology 同步；无凭证。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 流式窗口：subagent 之后的 Say 开流即让阶梯退场，subagent 行保持块序，Say standalone；无 step 命中目标、subagent 头在第 0 行 | `streaming_say_after_a_subagent_retires_the_ladder_behind_the_subagent_row` | `crates/tui/src/chat_tests/subagent_steps_continue.rs` |
| seal 后跨 subagent 行合并为 `▸ Say(3 steps): <preview>`，单行正文隐藏，`Done` 边界补恰好一个空行；展开阶梯后 subagent 行仍在正文前且记账对齐 | `sealed_say_after_a_subagent_merges_into_the_ladder_behind_the_subagent_row` | 同上 |
| 正文隐藏的合并对：阶梯紧邻 `last_block_ends_blank` 为真，隔 subagent 行为假；Marker 等其他块仍拆对 | `hidden_say_pair_behind_a_subagent_row_does_not_end_blank` | 同上 |
| 回放形状 `[SG, SG, Subagent, SG, Assistant]` 经 `coalesce_steps` 得 `[SG(3), Subagent, Assistant]`，与实时同渲染 | `replayed_ladder_behind_a_subagent_block_pairs_with_its_say_like_live` | 同上 |
| 命中记账镜像与 flatten 逐行对齐（含跨 subagent 配对） | `assert_line_accounting_matches` 全部调用方（新增文件内 4 处调用，每个用例一处） | `crates/tui/src/chat_tests/line_accounting.rs` |
| SPA：`[steps, subagent, say]` 归并为一个 assistantTurn，`say=[subagent, say]`，`progressActive` 冻结 | `absorbs a subagent block between the ladder and its Say: [steps, subagent, say] stays ONE bubble` | `crates/web/spa/src/bubbleItems.test.js` |
| SPA：run 之外的 subagent 块仍独立（之前 / `[steps, say, subagent]` 派发形状） | `keeps a subagent block standalone when it is not inside an open run` | 同上 |
| SPA DOM：同一气泡内顺序为 头行 → subagent 块 → Say 正文（done 与 sayActive 两种形态） | `keeps a subagent block between the ladder and its Say inside ONE bubble, in block order` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |

- 定向回归（全部通过，未运行全量测试）：
  - `cargo test -p opencoder-tui --lib` → 1849 passed / 0 failed（基线
    `6a5846ff`；flatten / `collect_headers` 为全 TUI 共用面，按直接影响链路
    跑完整个 lib 单测；新增 4 用例在修复前 3 红 1 绿，修复后全绿）
  - `cargo test -p opencoder-tui --lib chat::` → 257 passed / 0 failed
  - SPA `npx vitest run src/bubbleItems.test.js src/stepsBlock.dom.test.jsx
    src/subagentBlock.dom.test.jsx src/steps/reducer.test.js
    src/transcript/markdown.test.js` → 99 passed / 0 failed；
    `src/chat.dom.test.jsx` → 13 passed / 0 failed（合跑 112 passed；新增
    3 个 SPA 用例在修复前红）
- Clippy：`cargo clippy -p opencoder-tui --all-targets --no-deps -- -D warnings`
  → 零警告（不带 `--no-deps` 时在基线 `6a5846ff` 的 `opencoder-store` 自带
  4 处 lint 错误——`sessions/listing.rs` `single_char_add_str`、
  `sessions/parts/part_2.rs` `doc_lazy_continuation` ×3，属他人已提交改动，
  与本修复无关，未在此修）
- 测试范围：本次修改及其直接影响的链路；未运行全量测试。
