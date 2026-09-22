Commit: d9b366a66dc7defa4281f484dd00b2a3e208c092

# Agent 调度平台

Server/Node 调度、DAG 定义管理、执行查看与平滑发布；细节以代码为准。

## 执行准入与并发

- 节点磁盘可用块至少 10%、可用 inode 至少 20% 才接收新执行；容量读取失败或零容量拒绝准入，已有任务继续完成。
- DAG 定义的 `max_concurrency` 可配置为 1–30，缺省 4；画布和 JSON 编辑保存同一字段。每次运行冻结定义，修改定义不会改变在跑任务的上限。
- 兼容发布由当前 Server 信号触发独立作业；新任务转入新版本，既有任务保留所属 runtime。

## Operator 执行隔离

- 每个 Operator 执行获得独立的 HOME（冻结配置快照，0600）与 workspace；bash 工具 cwd 即 workspace，resume 重建同一对目录。
- Operator 配置来自节点数据根下的专用平面（首个执行引导一次后冻结），交互端（TUI/CLI）后续保存的配置与新增的全局技能包不再影响 Operator 执行；执行技能池 = 平面包 + 内置技能。
- 会话按创建时打上的 `kind` 泳道隔离（`operator`/`agent`/`team`/`dag`/`todos`/`project`/`brain`）；默认会话清单不显示 operator 泳道。

## DAG 私有任务文件

- `POST /api/executions` 的 `private_context` 承载有期限的执行专属文件，与公开 `input` 分离；节点能力接口为 `GET /api/nodes/{id}/execution-capabilities`。
- 调用方冻结 `image_digest`（运行中节点执行文件 SHA256）、`definition_sha256` 和 `expires_at_ms`。节点准入及恢复核验执行文件，私有文件变化不能复用旧执行 ID。
- host 使用 0700 私有目录和 0600 文件，runc 只读挂载 `/run/opencoder-task`；模型仅接收目录路径。DAG rootfs 安装脚本包含 Python 标准库运行时。
- 边界与回归映射见 [私有任务文件](../changelog/2026-09-22/private-dag-task-files.md)。

## 相关

- [协议与 API 明细](../../docs/agent-platform.md)、[平滑发布](../../docs/smooth-release.md)
- [动态 DAG 步骤](../../docs/dag-dynamic.md)
- [agents/control](../../agents/control/index.md) — 控制面与节点调度
- [agents/worker](../../agents/worker/index.md)、[agents/node](../../agents/node/index.md) — 节点执行与出站连接
