# 回退 rdb 会话存储：整段 revert 回 turso/SQLite

一句话：rdb（RocksDB）单写者 owner lock 使同一 workdir 无法同时开两个
opencoder 进程（第二个 TUI 启动即报 "already owned by another
process"）；整段 revert `a8b238c11..933300d73`（14 个提交），会话存储
完全回到 turso/SQLite WAL（`TursoStore::open(<data_dir>/opencoder.db)`），
多进程并发打开恢复。

## 动机

- RocksDB 单写者模型与"同机多终端多会话"的根本需求冲突；owner lock
  是设计行为而非 bug，无法在 rdb 内修补。
- turso/SQLite WAL 天然多进程共存，回退即恢复双开。
- 旧 `opencoder.db`（turso 库）在磁盘上完好且持续写到 rdb 接管前一刻，
  回退后无缝续用；rdb 时代写入的 `<stem>.rdb/` 目录成为死数据，回退后
  代码路径不再触碰，可手动删除。

## 变更

- `git revert a8b238c11^..933300d73`，零冲突：主存储引擎、路径策略、
  CLI/TUI 启动序列、ts 注册表、control definitions、worker 运行时
  全部回到 rdb 引入前（584207216）的实现。
- 随段回退的内容包括：legacy 迁移层删除提交（rdb clean cut）、Brain
  run 级 pause/resume/append（rdb 运行时存储）、definitions.db.rdb、
  rdb 写租约测试基座等 rdb 系列全部内容。
- Cargo 依赖回到段前状态：`rdb`/`rocksdb` 仅服务 event queue（本地
  path 依赖 `/root/rdb`），基础存储回归 turso。

## 验证

- `cargo check --workspace --tests` 干净；store 75 / local 91 /
  core 327 / tui open_store 定向测试全绿。
- 双进程并发打开同一 workdir 会话库无 owner lock 报错（turso WAL）。

## 遗留

- rdb 时代（约一天）产生的会话仅存在于 `*.rdb/` 目录，回退后不可读；
  如确认无需保留可删除 `opencoder.db.rdb*`（含 `.owner.lock`）。
