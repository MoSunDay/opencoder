# Turn / Step / Function call 三级层级纠正

Commit: (working-tree, 基于 f9b5581)

## 需求本质

- 顶层业务单位是一次 assistant Turn，而不是 reducer/消息存储产生的相邻片段。
- 默认态展示一个 `N steps` 摘要与 Say；流式更新不得擅自展开。
- 展开/收起是用户状态：reasoning、tool result、轮次结束、Compaction 最终摘要和本地 `!cmd` 结果等新输出只能更新内容，不能关闭或重开已有展开态；仅表头点击与 Ctrl+L/全局收起可改变它。
- 展开严格为 Turn → Step 内容 → Function call：Step 内显示 Thinking 与 `N function calls` 聚合行，打开聚合行后才列出 call，点击单个 call 只显示它自己的结果。
- `N steps` 后的运行动效表示“仍在等待 Say”：ToolEnd 不提前清除，首个非空 Say delta 或 Done/Error 才结束。TUI 间距两列，Web 间距 12px。

## 根因与修复

旧实现把“相邻 block”误当成 Turn 边界，因此 Say、marker 或 replay 消息对会制造多个 StepGroup；SPA 又靠渲染层把相邻片段视觉拼接，导致 live 与 replay 的 Step 数可能不同。现在 TUI live 以 `turn_block_start`、replay 以真实 User block 为唯一 Turn 边界；SPA live/snapshot 共用 `steps/reducer.js` 的 user-segment 规则。两端都在数据层保证一个 Turn 只有一个 StepGroup/steps item。

TUI 与 SPA 都保留 `N function calls` 作为 Step 内的第二层摘要；打开聚合行后才出现单个 call，call 的展开状态只控制自身结果。TUI 用 `progress_active`、SPA 用 `progressActive` 记录动效生命周期，而不再从“是否仍有未完成 call”推导。折叠 reasoning 的流式热路径仍只追加 `thinking_raw`，在 Step 首次打开或轮次收口时一次性渲染 Markdown，避免 O(n²)。

所有 disclosure 状态均与内容生命周期解耦。TUI 的 Step seal 只做 Markdown 物化与 token 记账，不再写 `open`；`finish_bash_tool` 只回填结果并停止动效；`finalize_compaction` 只替换最终摘要并结束 streaming。SPA 依赖稳定 Turn/call key 保留 Collapse 实例状态，流式 props 更新不重挂组件；只有 Ctrl/Cmd+L 或 `⤴ 收起` 递增 epoch 才统一重挂并收起。

## 需求—测试映射

| 需求 | 自动化证据 |
| --- | --- |
| 默认一个 Turn 只显示 `N steps + Say`，不自动展开 | `chat_tests/step_group/replay.rs::multi_round_turn_zero_click_shows_only_group_row_and_say`；`stepsBlock.dom.test.jsx` zero-click/streaming cases |
| Turn 展开只显示 Step | `chat_tests/tool_call_expand.rs::turn_click_reveals_only_steps`；`stepsBlock.dom.test.jsx` Turn click case |
| Step 展开只显示 Thinking + calls 聚合行 | `chat_tests/tool_call_expand.rs::step_click_reveals_thinking_and_calls_aggregate`；`stepsBlock.dom.test.jsx` Step click case |
| 聚合行列出 calls，单个 call 只展开自身结果 | `chat_tests/tool_call_expand.rs::calls_aggregate_reveals_rows_then_call_reveals_only_its_result`；`stepsBlock.dom.test.jsx` aggregate/call cases |
| 动效跨 ToolEnd 持续到 Say，终态收口 | `chat_tests/tool_collapse.rs::running_hint_persists_until_say_begins`；`reduce.test.js` progress lifecycle cases |
| Say/marker 不拆 Turn，真实 user 才拆 Turn | `chat_tests/tool_collapse/turn_routing.rs::text_between_calls_stays_in_one_turn_and_routes_by_id`；`chat_tests/step_group.rs::begin_turn_is_the_only_live_step_group_boundary`；`steps/reducer.test.js` cases (c2)/(c3)/(d) |
| TUI live/replay 与 SPA live/snapshot Step 语义一致 | `chat_tests/step_group/replay.rs::live_and_replay_share_one_turn_with_the_same_two_steps`；`steps/reducer.test.js::live SSE and snapshot replay produce the same...` |
| 隐藏 reasoning 不做逐 delta Markdown 全量渲染 | `thinking_state.rs::collapsed_live_reasoning_stays_raw_until_the_step_opens`；`app_loop_bugfix_tests/streaming_and_clock.rs::reasoning_deltas_render_first_frame_then_coalesce_hidden_updates` |
| 新输出保留已有展开/收起状态 | `chat_tests/step_group/disclosure.rs::new_output_never_closes_user_opened_ladder_levels`；`bash_tool.rs::finish_bash_tool_fills_output_without_closing_the_ladder`；`compaction_state.rs::compaction_delta_streams_into_expanded_block` / `compaction_updates_preserve_a_user_closed_stream`；`stepsBlock.dom.test.jsx::keeps user disclosure state when new output rerenders the turn` |
| Ctrl+L / 全局收起复位全部展开状态 | `chat_tests/tool_call_expand.rs::collapse_all_resets_all_three_levels`；`stepsBlock.dom.test.jsx` collapse-epoch case |

## 稳定上线门禁

- `cargo test --workspace`：全量 Rust 单元、集成与 doc tests 通过。
- `cargo test -p opencoder-tui --lib`：1625/1625 通过。
- `cd crates/web/spa && npm test`：15 个测试文件、150/150 通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo build --workspace` 与 `cargo build --release --workspace`：dev/release 均通过。
- `scripts/build-spa.sh` + `scripts/check-spa-drift.sh`：生产 bundle 重建并确认提交产物无漂移。Vite 仍报告既有的单 chunk 超过 500 kB 性能建议，不影响本次构建与功能验收。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。
