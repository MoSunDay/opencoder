Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# server 模块

版本 Server 与独立只读资源服务入口。

## 索引
- `crates/server/src/` — 二进制装配；复用 control/web 能力
- [main.rs](../../crates/server/src/main.rs) 的 `--data-dir` 指定 Server 持久化目录；执行过程由所属 Node 的同名参数指定。[rolling/config.py](../../scripts/platform/rolling/config.py) 的 `deployment.server_data` 与 `deployment.state_dir` 分别管理 Server 数据和版本运行、备份目录；参数及目录归属见 [部署说明](../../docs/agent-platform.md)、[发布配置](../../docs/smooth-release.md)。
- [control/release/resources.rs](../../crates/control/src/release/resources.rs) — 独立资源服务持有 Agent、Linux 二进制、源工作区与 Ontology 正文的只读 NFS 导出；经认证的资源管理接口与版本 Server 使用同一配置。资源进程不打开 Ontology 数据库，版本 Server 持有域状态并在退休前排空写入。
- 资源服务的 `GET /api/health` 返回 `role=resources` 与完整构建信息；兼容发布在预热前核对生产资源的版本。旧服务缺少元数据时要求维护升级。
- [rolling/maintenance](../../scripts/platform/rolling/maintenance/__init__.py) — 不兼容升级的持久阶段：冻结接入、排空、停服、备份、升级资源与挂载、启动迁移、私有验证、复开。迁移启动意图先落盘；复开写入意图持久化前可恢复旧库，此后禁止旧备份恢复，只允许同候选续跑或新格式版本修复。
- [maintenance/flow.py](../../scripts/platform/rolling/maintenance/flow.py) — 资源 HTTP 就绪后，按发布等待时限轮询导出状态和节点挂载的实际目录读取；成功后才准备候选进程。临时 NFS 读取错误可重试，超时保留 `installing` 阶段、关闭的写入口和原备份，同候选续跑继续核验这份备份。
- [maintenance/preflight.py](../../scripts/platform/rolling/maintenance/preflight.py)、[services.py](../../scripts/platform/rolling/maintenance/services.py) — 检查源工作区已存在且实际 Server 用户可读、可遍历；拒绝与可写管理路径重叠及软链接别名。资源升级仅置备应用目录，不创建或改动源目录。
- [maintenance/archive.py](../../scripts/platform/rolling/maintenance/archive.py)、[restore.py](../../scripts/platform/rolling/maintenance/restore.py) — 已停止写入并验证的 SQLite 备份使用只读 immutable URI，检查和恢复不会向备份添加 WAL/SHM 文件；活动数据库仍使用普通只读连接。
- [rolling/native/ontology.py](../../scripts/platform/rolling/native/ontology.py)、[rolling/backup.py](../../scripts/platform/rolling/backup.py)、[data_archive.py](../../scripts/platform/data_archive.py) — 从 ontology.db 读取实际正文根，备份数据库及正文树并核验引用摘要；维护恢复同时恢复这两部分，支持显式配置的外部正文根。
- [在线发布验收](../../scripts/acceptance/smooth_release/live.py) — 回滚、再发布和观察持续使用同一原生探针定义；等待标记须在任务所属 Runtime 的挂载及根目录视图中释放。[信号失败演练](../../scripts/acceptance/signal_release/main.py) 显式要求配套 `--rootfs`。

常规兼容发布保留资源服务与已有挂载；数据契约升级通过维护流程切换。参见 [发布说明](../../docs/smooth-release.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)。
