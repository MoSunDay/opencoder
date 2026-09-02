Commit: (working-tree)

# stats-sync 空库容错：0 字节 opencoder.db 断流 kaboo opencode 上报修复

## 背景

9/2 kaboo 看板整天没有 opencode 工具类型的新数据。追溯链：kaboo 以
`~/.local/share/opencode/opencode.db` 为 opencode 数据源，其 OpenCoder 会话
由 cron（每 30min）`opencoder-stats-sync-cron` →
`scripts/opencoder-to-opencode-stats.py` 增量写入；水位停在
2026-09-01T11:30 CST（`.opencoder-sync.json`），此后每 tick 都失败。

双故障叠加：

1. **0 字节空库炸全量**：OpenCoder 会为项目创建未初始化的 0 字节
   `opencoder.db`（无任何表，9/1 13:49 起持续新增，现存 11 个）。
   `collect_changes` 遍历逐库直查 `sessions`，首个空库即抛
   `no such table: sessions`，`main` 捕获后整体退出——一次坏库冻结整个
   水位，后续每 tick 原地崩溃。
2. **wrapper 路径失效**：`/usr/local/sbin/opencoder-stats-sync-cron` 仍指向
   `/data00/github/opencoder/scripts/...`，目录已更名 `opencoder-src`，
   即使脚本不崩也找不到文件。

## 实现

- `collect_changes`：逐库 `try/except sqlite3.Error`，坏库打 WARN 跳过，
  好库照常收割（19+/4-，含 `run` 内 `read_assistant_messages` 同型容错，
  跳过的 session 不推进 max_updated、下轮重试）。
- wrapper `script=` 指向 `opencoder-src` 新路径。

## 回归

- `scripts/test-stats-sync.py` 新增
  `test_collect_changes_skips_empty_db`（0 字节库 + 正常库并存，collect
  只取正常库会话），52 passed / 0 failed。
- 实机验证：修复后 sync 写入 231 sessions / 15437 messages（水位
  1788233293774 → 1788333777945）；kaboo 15:55 轮
  `parse.opencode entries 434462 → 450133`，上报
  `opencode 1922 buckets · 7229 sessions · 399 autonomy`（此前 6998/168），
  opencode 类型当日数据恢复上报。
