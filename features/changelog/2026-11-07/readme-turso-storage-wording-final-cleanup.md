# README 存储口径终清：rdb 会话存储时代措辞全部回归 Turso

一句话：rdb→turso 回退后的残留终清——README.md 中 11 处 RocksDB/rdb
口径（含整段存储对比表）恢复为 Turso/SQLite 现状；这批措辞源自 rdb
时代的记忆整理快照（9dacee1f7），早于 revert 范围 a8b238c11 起点而
未被整段回退带走，前一轮清理（见同日 turso-rebaseline-leftover-cleanup）
只修正了高性能行与死链，本次收口剩余全部。

## 变更清单

- `README.md` — 持久化层抽象描述、会话恢复、macOS 工具链注记、crate
  职责表 store 行、平台对比会话存储行、项目结构树 store 注释全部
  改回 Turso 口径；存储对比段整体恢复为 SQLite/Turso 驱动层差异表
  （含表模型、schema_version 幂等建表、workdir 库文件命名），Ontology
  注记改为独立 `ontology.db` 现状表述。
- `repo-memory.md` — Commit 指针刷新。

## 边界说明

- `crates/event` 的 `rdb` feature（RocksDB 持久队列账本）为现存独立
  可选功能，非会话存储残留，保持不动；workspace 中 rocksdb 仅该处使用。
- 历史 changelog 中提及 rdb 的条目为不可变历史记录，不回写。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 文档口径与代码一致（Turso=0.8.3、表清单、store_perf 链接） | 人工核验 | `README.md` |

- 测试范围：文档改动仅检查内容与链接，未运行构建或全量测试。
