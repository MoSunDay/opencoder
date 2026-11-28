# Subagent 段渲染在 Say 对上方：不切入 Say，段与 Say 之间空一行

一句话：TUI 里夹在 Step 阶梯与其 Say 之间的 subagent 段不再被压进合并后的 Say 块内部
（此前 `▸ Say(n steps): <preview>` 头行、subagent 行、正文三者相间，等于 subagent 把
Say 的内容切开），改为整体提到该对**上方**渲染——`> subagents` / `▸ subagents` 分组头、
subagent 行、一个空行，然后才是 `Say(n steps)` 头行与正文；subagent 下一行是 Say 时
（展开 / 收起、流式 / 已 seal 一致）都补这个空行。SPA 同症同治：
`transcript.jsx::AssistantTurnContent` 把 run 内 subagent 块渲染在气泡头行**上方**，
头行与 Say 正文保持连续。

## 背景与决策

- 用户反馈：subagents 不要贯穿了 Say 的内容；另外 subagent 下一行如果是 Say 需要空行。
- 根因：块序是 `[StepGroup, Subagent, Assistant]`，而合并头行由 `StepGroup` 臂输出、
  subagent 行由其后的 `Subagent` 臂输出，于是 seal 后 subagent 行落在头行与正文之间；
  流式窗口里 ladder 整体退场、subagent 行本来在 Say 上方，seal 瞬间布局跳位。
- 决策：渲染顺序与块序解耦。`chat_steps.rs::render_order` 把「阶梯与其 Say 之间的
  连续 Subagent 段」提到该阶梯之前（`say_after_group` 停在首个非 Subagent 块，故
  `group + 1..say` 恰为待提升段）；流式与 sealed 同序，seal 不再跳位。段后若下一块
  的首个渲染行是 Say（`opens_with_say`：standalone `Assistant`，或 follower 为
  `Assistant` 的 `StepGroup`——后者输出合并头行 / 流式时退场留下 Say 行），插入一个
  空行。折叠 / 展开、copy、hit-rect 全部走同一顺序。

## 行为与实现

- `crates/tui/src/chat_steps.rs`：新增 `render_order(blocks) -> Vec<usize>` 与
  `opens_with_say(blocks, bi)`，与 `say_after_group` / `group_before_say` 同为
  render 与 hit accounting 的单一事实源。
- `crates/tui/src/chat_flatten.rs`：`flatten_opts` 按 `render_order` 遍历；循环顶部在
  「上一块是 Subagent、当前块不是、且当前块以 Say 开头」时 push 一个空行；
  `last_block_ends_blank` 的合并对特例简化为恒 `true`——提升后的 subagent 行不再是
  对尾，闭合时头部后的分隔空行、展开时 ladder 自带的尾部空行都是最后一行，边界不叠
  第二个空行。
- `crates/tui/src/chat_headers.rs`：`collect_headers` 同序遍历并在同一边界
  `line_idx += 1`；`SubagentGroupHeader` / `SubagentHeader` / `ToolCallHeader` 的行号
  随之镜像。
- `crates/tui/src/chat_tests/line_accounting.rs`：期望行数镜像同步（`render_order` +
  边界空行），`assert_line_accounting_matches` 继续卡住两侧行数一致。
- `crates/web/spa/src/transcript.jsx`：`AssistantTurnContent` 把 `lead`（首个 Say 文本
  part 之前的 subagent 块）从正文容器移到 `StepsContent` 之前；`lead` 之后的块位置不变。
  收尾修正：`firstSay` 取首个 text part 而**不要求非空**——TUI 任何 TextDelta 都会开 Say 块
  （`chat_stream.rs`），首个 delta 为空白时 `[G, Sub, Assistant("\n")]` 已在 TUI 被提升；
  完全没有 text part（`[G, Sub]`）时两边都不提升，顺序逐窗口一致。
- `crates/web/spa/src/stepsBlock.dom.test.jsx`：顺序断言同步（block → 头行 → 正文，流式 /
  done 两态），并锁定段包装的 `marginBottom: 8`（段与 Say 之间的留白；jsdom 无布局，
  以样式为契约）。
- 构建：`npm run build`（`scripts/build-spa.sh` 的 vite build 步骤，跳过其 `npm ci`）刷新
  `crates/web/spa/dist`（内嵌产物与源码一致；dist 是共享工作树的整体构建，含同期未提交的
  其他 SPA 改动）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 流式窗口：段在 Say 上方，段与 Say 之间空一行 | `streaming_say_after_a_subagent_retires_the_ladder_behind_the_subagent_row` | `crates/tui/src/chat_tests/subagent_steps_continue.rs` |
| seal 后：段在合并 Say 对上方，头行与正文连续 | `sealed_say_after_a_subagent_merges_into_the_ladder_behind_the_subagent_row` | 同上 |
| 收起态仍保留段与 Say 的空行 | `collapsed_subagent_section_before_a_say_keeps_the_separating_blank` | 同上 |
| 非 Say 后继不补空行（块序保持紧凑） | `subagent_run_before_a_ladder_without_a_say_gets_no_blank` | 同上 |
| 隐藏正文合并对仍以空行收尾（Done 不叠空行） | `hidden_say_pair_behind_a_subagent_row_still_ends_blank` | 同上 |
| 回放块序与实时同形 | `replayed_ladder_behind_a_subagent_block_pairs_with_its_say_like_live` | 同上 |
| hit-rect 与 flatten 同序同数 | `assert_line_accounting_matches`（subagent / fold / render 各用例） | `crates/tui/src/chat_tests/line_accounting.rs` |
| SPA：段在气泡头行上方、头行与正文连续、段后留白 | `renders the subagent section above the ladder/Say pair inside ONE bubble` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |
| TUI：块序 `[Sub A, G, Sub B, Say]` 两段提升后合成一段（一个组头、两行、空行、合并对） | `subagents_on_both_sides_of_the_ladder_merge_into_one_section_above_the_say` | `crates/tui/src/chat_tests/subagent_steps_continue.rs` |
| SPA：空白 Say 仍提升（无 text part 时保持块序） | `hoists the subagent section above a whitespace-only Say (TUI opens the Say block on the first delta)` | `crates/web/spa/src/stepsBlock.dom.test.jsx` |

## 定向回归

- `cargo test -p opencoder-tui --lib subagent` → 98 passed / 0 failed；
  `--lib subagent_steps_continue` → 7 passed / 0 failed。
- 直接影响链路：`--lib say` 63、`step` 75、`render` 199、`line_accounting` 15、
  `flatten` 6、`tok_cost` 22、`live_region` 9、`collapse` 51、`clicking_` 4、
  `mouse` 39、`copy_mode` 41、`ctrl_l` 13、`thinking` 42、`hierarchy` 4、
  `dispatch` 45、`clear` 94、`transcript_reset` 9、`replay` 44、`reset_fold` 4、
  `sidecar` 50、`copy` 60 —— 均 0 failed。
- `npx vitest run src/stepsBlock.dom.test.jsx src/subagentFold.dom.test.jsx
  src/subagentBlock.dom.test.jsx src/bubbleItems.test.js src/chat.dom.test.jsx`
  → 5 files，82 passed / 0 failed。
- Clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告；
  `cargo fmt -p opencoder-tui -- --check` 无差异。
- 构建：`npm run build` → 成功（`dist/static/app.js` / `app.css` 刷新）。
- 变异验证：把 `lead` 临时挪回 `StepsContent` 之后、或把 `firstSay` 恢复为「非空」判据，
  各恰好只有对应 SPA 用例失败，恢复后 `cmp` 字节一致。
- 测试范围：本次修改及其直接影响链路；未运行全量 Rust / SPA 测试与浏览器验收。

## 收尾核实（评审遗留）

- **R2 dist 一致性**：`npm run build` 于收尾时重跑成功（9793 modules，49s），
  `dist/index.html` / `app.js` / `app.css` 刷新；此前 dist（23:07）已含本改动与同期的
  subagent 分组头改动，重跑只为兑现 R3 修正后的源码一致性。
- **R3 SPA 空 Say 瞬间（原 P3 假设）→ 成立，已修**：TUI 任何 TextDelta 都新建 `Assistant`
  块并立刻提升段；SPA 原 `firstSay` 要求 `trim().length > 0`，首个 delta 为空白（
  `appendDelta` 对 `"\n"` 会建 text part）的窗口内段仍在头行之下。修法为 lead 检测取首个
  text part；无 text part 时不提升（TUI 亦无 Say 块可配对），新增用例锁定两态。
- **R4 `[Sub A, G, Sub B, Say]`（原 P3 假设）→ 成立，行为可接受，已锁定**：`render_order`
  先发 `group + 1..say`（B）再发 G，渲染序为 `[A, B, G, Say]`，A、B 相邻共用一个
  `> subagents` 组头，随后空行 + 合并对；hit-rect 行号（组头 0、行 1/2、合并头 4）
  与 `assert_line_accounting_matches` 同步。
- **落地依赖（非逻辑缺陷）**：本改动与同期的 subagent 分组头 / 折叠开关改动在
  `chat_flatten.rs` 的 `flatten_with` 循环、`line_accounting.rs` 的 `prev_was_sub` 处交织，
  且本改动的用例断言 `> subagents` 与收起态，**不能单独提交**——须折叠开关先提交、本改动
  叠其上，`dist` 随第二个提交进入。

## Impact Surface

- TUI 转录：仅 `[StepGroup, Subagent, Assistant]` 这一块序的渲染位置变化——subagent
  段整体在 Say 对上方，段与 Say 之间多一个空行；其余块序、配对判定、Turn 边界、
  `turn_block_start` 推进规则均不变。
- TUI 交互：折叠 / 展开分组、点击 subagent 行进入视角、copy 文本、hit-rect 行号全部按
  新顺序镜像（`collect_headers` 与 `flatten_with` 同源）。
- Web：run 内 subagent 块渲染在气泡头行上方，其余 bubble / 折叠契约不变。
- 未改 API / CLI / 依赖 / Ontology 目录。
