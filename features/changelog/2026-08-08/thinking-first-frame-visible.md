Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# fix(tui): Thinking 标签在首个推理帧立即可见

## Context

LLM 层兼容 `delta.reasoning` 后能够持续产生 `ReasoningDelta`，但 TUI 在把事件应用到
`ChatView` 之后才判断末尾是否为折叠 Thinking。首个 delta 已经创建折叠块，因此也被当作
“隐藏内容增量”跳过绘制；若同一批次包含多个 reasoning delta，最后一个增量还会覆盖批次中
更早的可见变化。结果是模型确实正在思考，但用户要等到文本或其他事件到达后才看到标签。

## Change Summary

- `ChatView::last_open_thinking_collapsed()` 只识别仍开放的折叠 Thinking，排除历史 sealed 块。
- `fold_ui_events` 在应用 delta 前判断它是否只是向已显示的折叠块追加隐藏内容。
- 创建新 Thinking 块的首个 delta 强制允许下一帧绘制。
- 批次一旦包含任何可见变化，后续隐藏 reasoning 增量不能再次把整批标记为跳过。
- LLM client 的单元测试从超长 `client.rs` 拆到 `client_tests.rs`，生产文件恢复到 800 行以内。

## Validation

- 首个 reasoning delta：`skip_next_render = false`。
- 已显示折叠块的后续 delta：`skip_next_render = true`。
- 首个与后续 delta 同批到达：整批仍允许绘制。
- sealed Thinking 不被视为当前开放流。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
- [max/xhigh 推理字段兼容](max-xhigh-reasoning-thinking-label.md)
