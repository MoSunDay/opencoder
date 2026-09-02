# TUI sidecar 面板迁出块流：`SidecarPanel` + `Box<ChatView>`（不变量闭合，block_idx 首次稳定）

Commit: (working-tree)

## 背景

面板状态原本散在 blocks 流里（`ChatBlock::Sidecar` 变体）：fold、headers、render
三处都要穿 `blocks` 扫描找面板；面板开关期间的占位块 push/pop 使
`turn_block_start` 与 hit-rect 的 `block_idx` 漂移（steer 按钮 hit-rect 测试
直接受害）。把面板提为一等字段时遇到结构性问题：`Option<SidecarPanel>` 若内联
（面板含嵌套 `ChatView`，`ChatView` 又含面板）是无穷递归。

## 变更（crates/tui 9 文件，事件契约与 session 侧零改动）

- **`chat_types.rs`**：`ChatView` 新增 `sidecar: Option<SidecarPanel>` +
  `sidecar_focus: bool`；`SidecarPanel { id, question, view: Box<ChatView>, done,
  ok, answer, total_tokens, rounds, started_at_ms, elapsed_ms }`——`Box` 打断
  递归，derive（Default/Clone/Debug/PartialEq）对 `Box<ChatView>` 全部可用；
  删除 `ChatBlock::Sidecar` 变体（全仓 grep=0，无半迁移状态）。
- **`chat_sidecar.rs`**：`fold_sidecar` 全量改写 panel 字段。Start 双臂——
  adopt（空 id 占位原位收养，回显/嵌套 view 存活）与 replace（`panel.id` 非空
  时整面板替换，actor 侧重入竞态兜底）；前置 `!sidecar_focus` 门吞掉旧会话
  迟到 Start。Child/Turn 靠 `as_mut().filter(|p| p.id == *id)` 单次查找过滤。
  `purge` 两字段同清（focus 与 panel 原子成对）。
- **`sidecar_ui.rs`**：`enter_panel` 顺序 purge → 置 `Some` → 置 focus；
  `exit_panel` 对称双清；`echo_question` 直写 `panel.view`；`focused()` 只读
  借出 `(view, question, total_tokens)` 供 `compute_display` 换体。
- **消费方迁移**：`chat.rs` fold 路由、`app.rs` 面板内 follow-up（外层 else
  分支幂等重写）、`app_task.rs` `/task` 切换同清双字段、`app_loop.rs`
  `compute_display` 换 body/chip/ctx、`chat_headers.rs` Sidecar 臂删除、
  `app_display.rs`/`app_helpers.rs`/`app_submit.rs`。

## 不变量（评审闭合）

- `sidecar_focus == true ⇒ sidecar.is_some()`：4 个非测试写点全部成对成立。
- 晚到帧双保险：Start 靠 focus 门、Child/Turn 靠 id 过滤；`/task` 切换双清后
  旧会话 Start 被 focus 门吞；`echo_question` 对 `None` 自然 no-op。
- 重入竞态自愈：旧 Start 迟到收养空 id 占位 → 新 Start 到达 replace 替换面板
  （标题自愈），旧 id 后续 Turn 被 id 门控吞弃——旧版此时双块并存且旧块还会被
  迟到 Turn finalize，新版替换+吞弃与 destroy 契约（exit 即弃）一致。
- Busy 路径（channel 满）留空面板：空 question 只渲染导航标题，与旧行为同形。

## 兼容面（实证干净）

- `ChatBlock` 仅 `derive(Clone, Debug, PartialEq)` 无 serde；`ChatView` 从不
  序列化——replay 从持久化 `Message` + `SessionEvent` 重建。变体删除零持久化
  兼容风险；即便旧事件日志泄漏 Sidecar 帧，新视图 `sidecar_focus=false` 亦吞弃
  （与旧版同门）。
- `SessionEvent::Sidecar*` 契约与 session 侧 gate 零改动；单 crate 纯 UI 层重构。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 旧 Start 迟到后新 Start 整面板替换 | `second_start_replaces_the_panel` | `chat_tests/sidecar_fold.rs` |
| 面板关闭时 Start 吞弃 | `start_with_closed_panel_is_swallowed` | 同上 |
| 未知 id 的 Child 吞弃 | `sidecar_child_for_unknown_id_is_swallowed` | 同上 |
| 旧 id 迟到 Turn 不落新面板 | `sidecar_turn_finalizes_and_follow_ups_accumulate`（断言 stale id 零 token） | 同上 |
| 流式隔离组（新文件） | `chat_tests/sidecar_stream_isolation.rs` | 同目录 |
| block_idx 稳定化后 line_accounting 相应简化 | `chat_tests/line_accounting.rs` | 同目录 |

## 回归门

- `cargo test -p opencoder-tui`：lib 1602 passed / 0 failed，集成套件全绿。
- `cargo clippy -p opencoder-tui --all-targets` 0 告警；`cargo fmt --check` 清。
- 微瑕收敛：Child 臂与 Turn 臂统一为 `as_mut().filter` 单次查找；`app_loop.rs`
  注释 "sidecar block's" → "sidecar panel's" 文档漂移修正。
