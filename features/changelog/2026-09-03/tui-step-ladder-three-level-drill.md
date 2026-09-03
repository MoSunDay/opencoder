# TUI step 阶梯三级下钻重构 + thinking 步内吸收

Commit: (working-tree, 基于 2539b78)

## 背景

- 2539b78 的恒展开模型（静态 `≡ N steps` + 步行恒可见 + call 平铺 indent 4）在步数多时占屏过高；thinking 吸收只吃「严格尾随」think run——think → Say → tool 的轮次会把顶层 `💭 Thinking` 块留下来。
- 目标模型：工具轮顶层 = 一行可点击组行 `▸ N steps`（默认收起，点击展开；未完附 `⠋ running ` spinner）+ 终答 `❯ Say:` Assistant 块（永不折叠）。L0 组行 → L1 步行 `❯ Step(n)`/`▸ Step(n)`（indent 2）→ L2 步内 `💭 Thinking` + 聚合行 `▸ N function calls`（indent 4）→ L3 call 头/输出（indent 6，单 call 输出仍按 call 独立展开）。thinking 对工具轮步内吸收（跨该轮自己的 Assistant 块回溯，绝不顶层残留）；纯文本轮保持独立 Thinking 块。

## 变更

- 数据模型（`chat_types.rs`）：`StepGroup{steps, open}`（open 默认 false）；`Step{thinking, calls, open, calls_open}`（calls_open 默认 false）；新前缀常量 `GROUP_ROW_OPEN_PREFIX="❯ "` / `GROUP_ROW_CLOSED_PREFIX="▸ "`（含尾空格）；步行前缀沿用。
- 渲染（`chat_step_render.rs`）：`flatten_step_group(out, open, steps, anim_tick)`——组收起时整块 = 组行 + 尾随空行；聚合行 accent 加粗；任一 call `elapsed_ms==None` 时组行追加 spinner span（不加行）。
- thinking 吸收（`chat_steps.rs`）：`absorb_pending_thinking(blocks) -> Vec<Line>`（替代 `pop_trailing_thinking`）从尾部倒序走，跨 Assistant 块（透明、保留在流内），收集连续 Thinking 块（移除），遇 User/Marker/StepGroup/Subagent/Compaction/Image/Plan 即停；live `ToolStart` 使用之；replay `coalesce_steps` 以同边界语义回溯 `out` 后再并入 pending。倒序收集的 drop 索引先 reverse 再按升序重建（修复跨 Assistant 双 Thinking 只删一块的 bug）。
- 交互：`StepTarget{Group, Step(usize), Calls(usize), Call(usize,usize)}` 四路分发（`chat.rs::toggle_tool_call_at`）；`visible_targets(open, steps)` 与渲染同构；`collapse_all_collapsible`（Ctrl+L）收 group open + step open + calls_open + expanded 四级；`chat_headers.rs::collect_headers` 三级行数核算镜像（组行恒为目标）；`render_hits.rs` 命中记录同步。
- copy 模式（`copy_mode/clean.rs`）：组行匹配改 `{❯|▸} N step(s)`（共享 `count_row_label`，spinner span 尾缀 `running `）；新增 `LineKind::CallsRow`（`{❯|▸} N function call(s)`，indent 4 gutter 剥离后匹配）——组行/步行/聚合行/thinking 头均为 chrome 丢弃，call 头（indent 6）与输出保留为内容。
- `chat_helpers.rs`：`push_bash_tool`（`!cmd`）四级全开进入 Results 态，`finish_bash_tool` 四级全收。

## 测试清单（规则 01/02：每行为有测试）

- `chat_tests/step_group.rs`：零点击仅组行+Say；跨 Assistant 的 thinking 步内吸收；纯文本轮独立 Thinking；replay coalesce 同语义；copy 文本丢弃组/步/聚合行/thinking 头、保留 call 头与输出。
- `chat_tests/tool_call_expand.rs`：四目标全链路开合、阶梯形状（8 行）、兄弟隔离、Ctrl+L、越界 no-op、headers walk。
- `chat_tests/tool_collapse.rs`：收起默认单组行（2 行）、逐级展开 2→4→5→6→8→2、collapse_all 覆盖工具轮 + 纯文本轮、组行/步行 header_line_idx 落点（含 running spinner hint）。
- `chat_tests/line_accounting.rs`：三级行数核算镜像（收起 2 行 / 每深度 4/6/11 行）、展开 call 平移、running spinner span 不加行、mixed 序列组行落位 line 16。
- `app_helpers_tests/mouse_tests.rs`：click walk [Group]→[G,S1,S2]→…→Call b，四级互不串扰；组行再点收起且不重置内层状态。
- `app_helpers_tests/ctrl_l_tests.rs`：父/子 view 四级全开后 Ctrl+L 全收断言。
- `copy_mode/clean.rs` 单测：组行新旧字形/单复数/spinner/三类 lookalike 存活、CallsRow classify（slot 4）与丢弃。
- `chat_steps.rs` 单测：`visible_targets` 三级镜像、absorb 跨 Assistant、user 边界停。
- 全量回归：`cargo test -p opencoder-tui` lib 1617 + tests 27 个二进制 0 failed；`cargo check --workspace` 通过；`cargo fmt --all` 已跑。

## 兼容

- 旧 `≡ N steps` 静态标与组级 `toggle_step_group_at` 已不存在；点击目标索引语义变化（0 现为组行），`render_hits`/鼠标/headers 全部同步。
- Web/SPA 侧同步重构见 [spa-step-ladder-three-level-drill.md](spa-step-ladder-three-level-drill.md)；上一代恒展开模型见 [step-ladder-unfold-tui-spa.md](step-ladder-unfold-tui-spa.md)。
