Commit: (working-tree, pre-initial-commit)

# go-live review 修复：cargo fmt + compact() no-op 误报

## 背景
go-live review 发现两个问题：
1. `cargo fmt --check` 有 544 处违规（全仓库从未格式化）
2. `compact()` 在 `split==0`（消息不足以压缩）时返回 `Ok("nothing to compact")`，但 runner 仍 emit `TranscriptReset` + `Compaction("nothing to compact")` 事件——造成 TUI 无意义重建 + CLI 误导性日志

## 变更

### A — `cargo fmt --all` 全仓库格式化
- 544 处格式违规全部修复
- `cargo fmt --all --check` 退出 0

### B — `compact()` 返回 `Option<String>` 消除 no-op 误报
- **`compaction.rs`**：`compact()` 签名从 `Result<String>` 改为 `Result<Option<String>>`。`split==0` 时返回 `Ok(None)` 而非 `Ok("nothing to compact")`
- **`runner.rs`**：`match` 增加 `Ok(None) => {}` 分支——不 emit 任何事件
- **`worker.rs`**（TUI `/compact` 手动触发）：同步修改，`Ok(None) => {}`
- **`scripts/e2e-glm.sh`**：E3 检查简化——不再需要 `grep -v 'nothing to compact'` 过滤假阳性

## 涉及文件
- 全仓库 `*.rs` — cargo fmt 格式化
- `crates/session/src/compaction.rs` — `compact()` 返回类型改 `Option`
- `crates/session/src/runner.rs` — `Ok(None) => {}` 分支
- `crates/tui/src/worker.rs` — `Ok(None) => {}` 分支
- `scripts/e2e-glm.sh` — E3 简化 grep

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| fmt | 544 violations | clean |
| test | 276 passed | 285 passed |
