Commit: f23063530a72e73bddef57a2d077c39754b16e29

# server 模块

版本 Server 与独立只读资源服务入口。

[main.rs](../../crates/server/src/main.rs) 的 `--auth-enabled true|false` 默认开启公共 HTTP 鉴权；部署配置 [rolling/config.py](../../scripts/platform/rolling/config.py) 接受布尔 `deployment.auth_enabled`，由新 Server unit 传入。开关不轮换或撤销 Token；资源服务与 Runtime 继续使用现有机器凭据。操作见 [项目 CLI 与鉴权](../../docs/project-cli.md)。

## 索引
- `crates/server/src/` — 二进制装配；复用 control/web 能力
- [main.rs](../../crates/server/src/main.rs) 的 `--data-dir` 指定 Server 持久化目录；执行过程由所属 Node 的同名参数指定。[rolling/config.py](../../scripts/platform/rolling/config.py) 从唯一 `deployment.state_dir` 派生 Server 数据与服务工作目录；[rolling/storage](../../scripts/platform/rolling/storage/__init__.py) 统一生成角色资源路径和只读挂载配置，检查外部源工作区、目录别名与 Ontology 绑定，拒绝覆盖已有不同配置。目录归属见 [发布配置](../../docs/smooth-release.md)。
- 部署生成的 Server `agent.share_dir` 固定为 `server/.opencoder/share`，准备时检查路径并赋予 Server 用户读写权限；环境、工具、TODO 模板与能力列表共用此目录，不依赖服务 HOME，也不将此路径继承给执行节点。
- [control/release/resources.rs](../../crates/control/src/release/resources.rs) — 独立资源服务持有 Agent、Linux 二进制、源工作区与 Ontology 正文的只读 NFS 导出；经认证的资源管理接口与版本 Server 使用同一配置。资源进程不打开 Ontology 数据库，版本 Server 持有域状态并在退休前排空写入。
- 资源服务的 `GET /api/health` 返回 `role=resources` 与完整构建信息；兼容发布在预热前核对生产资源的版本。旧服务缺少元数据时要求维护升级。
- [rolling/maintenance](../../scripts/platform/rolling/maintenance/__init__.py) — 不兼容升级的持久阶段：冻结接入、排空、停服、备份、升级资源与挂载、启动迁移、私有验证、复开。迁移启动意图先落盘；复开写入意图持久化前可恢复旧库，此后禁止旧备份恢复，只允许同候选续跑或新格式版本修复。
- [maintenance/runtimes.py](../../scripts/platform/rolling/maintenance/runtimes.py) — 排空检查保留 idle、interrupted 的 Agent/Operator 会话；必须同时确认可休眠、运行数、队列和所属进程均为空，不改写原会话状态或历史。
- [maintenance/flow.py](../../scripts/platform/rolling/maintenance/flow.py) — 资源 HTTP 就绪后，按发布等待时限轮询导出状态和节点挂载的实际目录读取；成功后才准备候选进程。临时 NFS 读取错误可重试，超时保留 `installing` 阶段、关闭的写入口和原备份，同候选续跑继续核验这份备份。
- [maintenance/preflight.py](../../scripts/platform/rolling/maintenance/preflight.py)、[services.py](../../scripts/platform/rolling/maintenance/services.py) — 检查源工作区已存在且实际 Server 用户可读、可遍历；拒绝与可写管理路径重叠及软链接别名。资源升级仅置备应用目录，不创建或改动源目录。
- [maintenance/archive.py](../../scripts/platform/rolling/maintenance/archive.py)、[restore.py](../../scripts/platform/rolling/maintenance/restore.py) — 已停止写入并验证的 SQLite 备份使用只读 immutable URI，检查和恢复不会向备份添加 WAL/SHM 文件；活动数据库仍使用普通只读连接。
- [rolling/native/ontology.py](../../scripts/platform/rolling/native/ontology.py)、[rolling/backup.py](../../scripts/platform/rolling/backup.py)、[data_archive.py](../../scripts/platform/data_archive.py) — 从 ontology.db 读取实际正文根，备份数据库及正文树并核验引用摘要；维护恢复同时恢复这两部分，支持显式配置的外部正文根。
- [backup_files/scope.py](../../scripts/platform/rolling/backup_files/scope.py) — 在线 Runtime DB 扫描在读取子目录前排除固定资源、OCI 镜像及有回执且根只读的 DAG 容器视图；已知 `.cleared-executions-*` 归档同样处理。真实 HOME、workspace、写层和项目 DB 保留，排除路径写入备份清单；停服备份不使用该排除规则。
- [rolling/probes.py](../../scripts/platform/rolling/probes.py) 在正向发布及回滚激活 Host 后，按 `/status` 的节点 ID 与 generation 等待目标 Server 上对应节点在线、索引同步后就绪，再切换 Nginx。其他在线节点或旧 Host 的就绪状态不能替代此次交接。
- [在线发布验收](../../scripts/acceptance/smooth_release/live.py) — 回滚、再发布和观察持续使用同一原生探针定义；等待标记须在任务所属 Runtime 的挂载及根目录视图中释放。[信号失败演练](../../scripts/acceptance/signal_release/main.py) 显式要求配套 `--rootfs`。

常规兼容发布保留资源服务与已有挂载；数据契约升级通过维护流程切换。参见 [发布说明](../../docs/smooth-release.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)。

TODO 文件导出由 `--todo-resources` 独立进程负责，只提供健康及 TODO NFS 管理接口，使用启动凭证认证，不打开控制数据库。既有 `--resources` 不托管此导出。
