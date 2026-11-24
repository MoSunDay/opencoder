# Turso 存储库修库（dump + 页级 carve 重建）

`opencoder.db`（Turso 0.8 存储引擎，SQLite 文件格式）出现 `IdxDelete: no matching index entry`、`FOREIGN KEY constraint failed`、`inconsistent overflow chain`、按 rowid 读取 `messages` 报 `database disk image is malformed` 时，说明表 B-tree 与索引已经失步甚至页被覆盖，应用层无法绕过，只能重建文件。该工具在**副本**上工作：

1. 把 `opencoder.db`（含 `-wal`）复制到输出目录旁的 `.repair-<name>/` 临时目录，源库从不被写入；
2. 用 `sqlite3` 命令行 `.dump` 导出可读内容（WAL 会被折叠进副本；`messages` 这类根页已损坏的表导出为 0 行，这是预期现象）；
3. 把结尾的 `ROLLBACK; -- due to errors` 改为 `COMMIT;`，并把 `INSERT INTO` 改写为 `INSERT OR REPLACE INTO`（同一主键的陈旧页副本不再让导入中断），`sqlite3 -bail` 导入新文件；
4. 对副本做一次全页扫描（只读、mmap）：凡单元记录符合 `messages`（14 列）/`session_events`（6 列）形状的叶子单元，按原 `seq`（rowid）补回新库；位于索引叶页、没有 rowid 的 `messages` 行按 `created_at` 顺序继承旧索引里尚未使用的 `seq`；
5. 消息仍在但 `sessions` 行已丢失的会话补一条最小会话行（保持外键有效）；`task_type` 为二进制垃圾、`kind` 为空串的会话行修正为 `parent`/`subagent`/NULL；
6. `sqlite_sequence` 抬到各表最大 `seq`，`PRAGMA integrity_check`，输出 JSON 报告。

需要带 `sqlite3` 模块的 Python（若默认 `python3` 缺少 `_sqlite3`，改用带该模块的解释器，例如 `/usr/bin/python3`）以及 `sqlite3` 命令行。以下命令在仓库根目录执行。

```sh
# 0. 先停掉全部 opencoder 进程（多进程同时写同一库正是损坏来源之一）
pgrep -a opencoder

# 1. 备份整个数据目录（保留 engine.json / .rdb / ontology / project / server-v2）
DATA=~/.local/share/opencoder/<workdir-hash>
mkdir -p ~/opencoder-db-backup && cp -a "$DATA"/opencoder.db* ~/opencoder-db-backup/

# 2. 重建到临时目录（耗时随库大小增长，数 GB 的库约十分钟；临时目录需预留约两倍库大小的空间）
mkdir -p /tmp/opencoder-repaired
PYTHONPATH=scripts/maintenance /usr/bin/python3 -m db_repair \
  --database "$DATA/opencoder.db" \
  --output   /tmp/opencoder-repaired/opencoder.db \
  --json     /tmp/opencoder-repaired/report.json

# 3. 核对报告：verify.integrity 必须是 ok；carved.messages_with_seq 应接近损坏索引条目数；
#    stub_sessions / normalized_sessions 列出被补行/修正的会话。

# 4. 替换前再次确认没有进程在跑（opencoder-server 之类的服务也要先停）
pgrep -a opencoder

# 5. 替换：旧库保留为 .corrupt-*；-wal/-shm/-tshm 属于旧库，必须在新库就位之前删除
#    （旧 WAL 若被当成新库的日志回放，会把损坏页写回新库）
mv "$DATA/opencoder.db" "$DATA/opencoder.db.corrupt-$(date +%s)"
rm -f "$DATA"/opencoder.db-wal "$DATA"/opencoder.db-shm "$DATA"/opencoder.db-tshm
cp /tmp/opencoder-repaired/opencoder.db "$DATA/opencoder.db"

# 6. 启动 opencoder；首次打开会执行 schema 迁移并重建会话活跃索引。
```

说明：

- 导入报告里的 `carved.messages_without_seq` 是只残留在索引叶页、表行已丢失的消息，它们拿到的 `seq` 只保证会话内时间顺序正确；`index_only` 条目对应的行若页已被覆盖则无法恢复。
- 该工具不会修改源库：`--output` 指向 `--database` 同一文件时直接拒绝；运行中途失败会保留 `.repair-<name>/` 里的副本与 dump（日志给出路径），成功时才删除（`--keep-workdir` 可强制保留）。
- 无 rowid 的索引叶页行按**所属会话**继承旧索引里尚未使用的 `seq`（会话内按 `created_at` 顺序对位）；索引里已没有该会话条目的行拿到高于现有最大值的新 `seq`。
- `--dump` 可复用同一库状态的既有 `.dump` 文件（会被原地改写）。
- 修库不能消除根因：多进程 `experimental_multiprocess_wal` 并发写同一文件导致的页级丢失更新会再次损坏，应减少并发或升级存储引擎。

定向测试：`PYTHONPATH=scripts/maintenance /usr/bin/python3 -m unittest db_repair.test_repair -v`。
