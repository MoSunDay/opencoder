Commit: (working-tree, pre-initial-commit)

# compact 清屏 + session 二进制导出/导入

## 变更

### compact 后清空展示内容
- **新增 `SessionEvent::TranscriptReset(Vec<Message>)`**：compaction 成功后携带新的消息列表，display 层据此重建视图。
- **runner auto-compact**（`runner.rs:106-111`）：compact 成功后先发 `TranscriptReset(msgs)` 再发 `Compaction(summary)`。
- **worker 手动 `/compact`**（`worker.rs:69-81`）：同上。
- **app.rs 事件循环**：收到 `TranscriptReset` 时调用 `replay_into_chat` 重建 `chat.blocks`（清空旧内容，重建为 [summary, tail...] 视图），不调 `chat.apply`。
- **chat.rs/cli/run.rs/web/handle.rs**：新增 `TranscriptReset` match arm（no-op 或映射）。
- 效果：compact 后显示内容清空，仅展示压缩后的消息摘要 + 尾部对话 + `[context compacted]` marker。

### session 二进制导出/导入（含 subagent 树）
- **自定义 opencode 二进制格式**（`.opencode` 扩展名）：8 字节 magic `OPENCODR` + 4 字节版本 LE + 8 字节 payload 长度 LE + serde_json payload。零新依赖。
- **新增 `crates/store/src/bundle.rs`（~190 行）**：
  - `SessionBundle { meta, messages, events, inputs, subagents }` — 递归结构。
  - `SubagentBundle { task, child: SessionBundle }` — 子 agent 完整数据。
  - `export_bundle(store, id)` — 递归收集 session + 子 agent。
  - `import_bundle(store, bundle, workdir_hash)` — 递归写入；幂等（session 已存在则跳过）。
  - `write_bundle(bundle, writer)` / `read_bundle(reader)` — 二进制序列化。
  - 3 个单元测试 + 1 个集成测试（round-trip with subagent tree）。
- **CLI 命令**：
  - `opencoder session export <id> [-o file]` — 导出到 `<id>.opencode`。
  - `opencode session import <file>` — 导入并打印 session id，提示 `--session <id>` 继续。
- **不导出 Config**（含 API key，安全）。
- **导入后可继续执行**：`opencode --session <id>` 或 TUI `/task` 选中即可通过现有 `resume` 路径恢复。

## 涉及文件
- `crates/session/src/runner.rs` — 新增 `TranscriptReset` 变体 + auto-compact 发送
- `crates/tui/src/worker.rs` — 手动 compact 发送 `TranscriptReset`
- `crates/tui/src/app.rs` — 事件循环 `TranscriptReset` → `replay_into_chat` 重建
- `crates/tui/src/chat.rs` — 新增 no-op arm
- `crates/cli/src/run.rs` — 新增 no-op arm
- `crates/web/src/handle.rs` — 新增映射 arm
- `crates/store/src/bundle.rs` — **新增**，~190 行
- `crates/store/src/lib.rs` — `pub mod bundle` + re-exports
- `crates/cli/src/lib.rs` — `SessionSub` 新增 `Export`/`Import`
- `crates/cli/src/session_cmd.rs` — dispatch + 导入逻辑
- `crates/store/tests/store_integration.rs` — `bundle_export_import_roundtrip` 测试

## 测试
- 全量：234 passed, 0 failed, clippy --all-targets -D warnings clean
- 新增：`bundle::round_trip_binary`、`rejects_bad_magic`、`rejects_wrong_version`、`bundle_export_import_roundtrip`
- E2E：export 0-subagent session ✓；export 1-subagent session ✓；import 到新 workdir ✓；`session list` 可见 ✓；`session show` 内容完整 ✓
