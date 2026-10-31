Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# server 模块

`opencoder-server` 舰队控制面二进制：Web 控制台、全局定义与调度，不做本地执行。

## 索引

- [main.rs](../../crates/server/src/main.rs) — 二进制装配与启动参数
  （`--web`、`--auth-enabled`、`--data-dir`、`--resources`、`--todo-resources`）
- 能力复用 [control](../control/index.md)、[web](../web/index.md)；只读资源与 TODO
  导出见 [release/resources.rs](../../crates/control/src/release/resources.rs)、
  [release/todo_resources.rs](../../crates/control/src/release/todo_resources.rs)
- 发布部署：[rolling/config.py](../../scripts/platform/rolling/config.py)、
  [rolling/storage](../../scripts/platform/rolling/storage/__init__.py)、
  [rolling/maintenance](../../scripts/platform/rolling/maintenance/__init__.py)
- 备份与数据归档：[rolling/backup.py](../../scripts/platform/rolling/backup.py)、
  [data_archive.py](../../scripts/platform/data_archive.py)、
  [backup_files/scope.py](../../scripts/platform/rolling/backup_files/scope.py)
- 就绪探针 [rolling/probes.py](../../scripts/platform/rolling/probes.py)；发布验收
  [smooth_release/live.py](../../scripts/acceptance/smooth_release/live.py)、
  [signal_release/main.py](../../scripts/acceptance/signal_release/main.py)
- 文档：[项目 CLI 与鉴权](../../docs/project-cli.md)、
  [发布配置](../../docs/smooth-release.md)

数据契约升级走维护流程；参见
[DAG 执行约定](../../rules/04-dag-execution-contract.md)。
