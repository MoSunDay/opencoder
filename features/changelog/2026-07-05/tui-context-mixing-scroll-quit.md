Commit: (working-tree, pre-initial-commit)

# TUI 修复四则：上下文统计 / 文本工具混排 / 滚动跟随 / Ctrl+D 退出

## Context
迭代中用户报告四个 TUI 缺陷：①系统提示词未计入 ctx%；②机器人正文与 bash 输出混排在一起；③滚动条位置不对且「跟随中…」实际未跟随到底；④Ctrl+D 无法退出。均为 `crates/tui` 渲染与键处理的既有问题，与 skill 选择器无关。

## Change Summary
- **①系统提示词计入上下文（`app.rs`）**：新增 `sys_tokens_for(agent_name, workdir, skill)` —— 经 `opencoder_session::prompt::build_system(...).text()` + `estimate` 估算系统提示词（agent.prompt + 环境块 + 当前 skill）token；`run_app` 初始化 `sys_tokens`，在 `SwitchAgent` / `SetSkill` 分支即时重算。`render_status` 显示 `context_used + sys_tokens`。`context_used`（转录流式累计、压缩时重置）与 `sys_tokens`（系统提示词，跨压缩常驻）分离，使百分比反映真实请求体积。
- **②正文/工具混排（`chat.rs`）**：根因 `push_raw` 无条件把 TextDelta 续到最后一行 —— 工具输出后的下一句正文被粘到 bash 行尾。`ChatView` 增 `merge_next: bool`：仅当上一事件是「未以换行结尾的 TextDelta」时才续行；所有非 TextDelta 事件（ToolStart/ToolEnd/Subagent*/AgentSwitch/Compaction/Done/Error）置 `merge_next=false`，保证正文总在新行起笔。重写 `push_raw`（按 `\n` 切片、剔除末尾换行、bare 换行渲染为空行）。`ToolEnd` 后再 push 一空行作视觉分隔。
- **③滚动条与跟随（`chat.rs` + `app.rs`）**：根因 `render_body` 用「逻辑行数」算 `max_scroll`，但 `Paragraph` 启用 `Wrap`，长行占多屏行 → `max_scroll` 偏小 → follow 到不了底、滚动条 thumb 错位。新增 `wrapped_metrics(lines, width) -> (rows_per_line, total_rows)`（每行 = `max(1, ceil(Line::width()/w))`）、`tail_offset(rows, visible_h)`（尾部窗口起始行）、`row_offset_at(rows, idx)`（累计行）。`render_body` 改用包裹行算 `pos_line`（follow = `max_line` 钉底，手动 = `scroll.clamp(max_line)`）与 `ScrollbarState.position`。鼠标滚轮向下「到底重新跟随」的阈值同步改用 `tail_offset`。
- **④Ctrl+D 退出（`app.rs`）**：既有 `Char('c')|Char('d') + CONTROL` 已处理，但部分终端/crossterm 配置把 Ctrl+C/Ctrl+D 以裸控制字符（ETX 0x03 / EOT 0x04，无 CONTROL 位）投递，漏过 Ctrl 分支。在 `Char(c)` 分支开头补 `if c=='\u{3}' || c=='\u{4}' { return Quit; }` 兜底。

## Impact Surface
- 修改：`crates/tui/src/chat.rs`（`merge_next` 字段 + `push_raw` 重写 + 非文本事件 reset + ToolEnd 空行 + 三个 metrics 函数）、`crates/tui/src/app.rs`（`sys_tokens` 状态与重算、`render_body` 改用包裹行、鼠标滚轮阈值、Ctrl+D 兜底）、`crates/tui/src/app_tests.rs`（+3 退出测试）。
- 新增测试 9 条（chat 6：正文/工具分行、续行、多行切片、wrapped_metrics、tail_offset、row_offset；app 3：ctrl_d / raw_eot / raw_etx）。全量 159 passed、`clippy --all-targets -- -D warnings` 全绿。
- 行为契约：ctx% 现包含系统提示词；转录中正文与工具块以空行分隔且不再相互吞并；「跟随中…」真正钉到底、滚动条 thumb 反映真实包裹行；Ctrl+D/Ctrl+C 在两种投递形态下都能退出。

## Notes / Compatibility
- `CONTEXT_BASELINE=4_000`（`fmt::context_percent` 仍从中减去）未动；系统提示词现显式计入 `used`，绝对值 `(used/limit)` 已含它。若希望小会话也立刻非零，可单独下调 baseline——未在本改动范围内。
- `render_paragraph` 仍 clone 全量 `lines`（既有 O(n)），`wrapped_metrics` 每帧再 O(n) 遍历；超长转录可能略增开销，但转录用例下可接受，缓存化留作后续优化。
- 包裹行计算依赖 `Line::width()`（ratatui 0.29 unicode 宽度）；CJK 双宽已被计入。

## Follow-up（上线前 review 修复）
- **正文/工具混排回归（②补全）**：①的 `merge_next` 是 `chat.rs` 私有字段，但 `app.rs` 有 5 处直接 `chat.lines.push(...)`（`push_user` ×2、Submit-while-running `[queued]`、`Steer`、`Queue`、`Cancel`）绕过了它——运行中 Steer/Queue 之后到达的 TextDelta 会被粘到 marker 行，重新引发混排。新增 `ChatView::push_marker(line)`（push 前置 `merge_next=false`），全部 6 处改走它；回归测试 `push_marker_then_text_starts_fresh_line`。
- **③滚动单位修正（关键）**：原实现把「逻辑行索引」喂给 `Paragraph::scroll((y,0))`，但读 ratatui 0.29 源码（`paragraph.rs::render_text`）证实 `scroll.y` 单位是**包裹后的显示行**（word-wrapper 每次 `next_line` 产出一行、`y` 按包裹行自增）——故一旦有换行，滚动位置就错位、follow 钉不到底。改为喂**包裹行偏移**：follow 时 `scroll = total_rows - visible_h`（恰好填满视口、末行钉底），手动滚动同样以包裹行计。`render_body` 收 `&mut scroll` 每帧同步（follow 钉底 + 钳制），修正了「PageUp 从 follow 起跳用陈旧值」的副作用。
- **③包裹行计数精确化（上线前二轮 review）**：初版用自写的 `wrapped_metrics`（`ceil(line_width/cols)`）估算 `total_rows`，但 ratatui 在**词边界**处换行（`WordWrapper`），词间空格会让实际行数**少于**字符除法——导致 follow 滚过头、底部留白。改用 ratatui 自带的 `Paragraph::line_count(width)`（内部跑同一个 `WordWrapper`，计数与渲染完全一致），启用 `unstable-rendered-line-info` feature。删除 `wrapped_metrics`/`render_paragraph` 及其测试，新增 `line_count_matches_word_wrap_not_char_division`（证明词边界换行 < 字符除法）+ 升级 `paragraph_scroll_uses_wrapped_rows_and_pins_tail` 改用 `line_count`。`render_body` 与鼠标滚轮 handler 均改走 `line_count`。
- **`sys_tokens_for` 补测**：`pub(crate)` 化 + 单元测试（act>0、确定性、skill 增量、未知 agent→0）。
- 全量 160 passed，`clippy --all-targets -- -D warnings` 全绿。Release 二进制 9.5 MB（stripped），已安装至 `/usr/local/bin/opencoder` + `/root/.cargo/bin/opencoder`。
- 未处理（已知、非阻断）：每帧 O(n)（`line_count` + `lines.clone()`，建议按 `(len,width)` 缓存）、`dirty` 字段写而不读、`CONTEXT_BASELINE` 仍遮蔽小会话百分比、`scroll_y as u16` >65535 包裹行截断。

## Related Docs
- [agents/session](../../../agents/session/index.md) — 系统提示词由 `build_system` 组装。
- [skill-picker changelog](./skill-picker.md) — skill 注入亦经 `build_system`，其体积现计入 ctx%。
