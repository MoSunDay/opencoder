Commit: d9b366a66dc7defa4281f484dd00b2a3e208c092

# DAG 私有任务文件交付

`POST /api/executions` 可独立提交 `private_context`，仅适用于 DAG。公开 request/input 与查询结果不包含私有文件。
`GET /api/nodes/{id}/execution-capabilities` 返回节点能力与运行中执行文件 SHA256；调用方冻结节点、定义摘要和期限。
服务端及节点校验定义，节点核验执行文件；私有输入参与重试身份判断，不能以同执行 ID 替换凭证。

文件保存在节点执行专属 0700 目录，文件 0600，写入后校验且不可覆盖。节点 journal 使用 0600 文件。
DAG host prompt 仅包含路径；runc 只读挂载 `/run/opencoder-task`。凭证由工具读文件，不进入模型输入。
DAG rootfs 准备补齐 Python 标准库与动态依赖，并用 chroot 核验采集器所需运行模块。
这是代码与隔离验证，本次未发布。

| 功能 | 测试 |
| --- | --- |
| 私有字段不进入公开 request | `private_submission_fields_do_not_enter_public_request` |
| 期限、路径、容量限制及 Debug 脱敏 | `task_grant_validates_expiry_bounds_paths_and_redacts_debug` |
| 权限、不可变重放、符号链接拒绝 | `materialization_is_private_immutable_and_replayable`、`private_file_symlinks_are_rejected` |
| 普通能力探测不读取执行文件；私有探测异步计算摘要并使用独立超时窗口 | `only_private_file_probes_use_the_digest_window`、`private_dag_files_are_pinned_durable_and_absent_from_public_readback` |
| runc 只读挂载和路径 prompt | `task_mount_is_read_only_and_prompts_include_paths_only` |
| 真实 Worker 接收、执行、重启、漂移拒绝与公开读回隔离 | `private_dag_files_are_pinned_durable_and_absent_from_public_readback` |
| Python rootfs 运行依赖 | `scripts/dag-rootfs/install-python.sh` 内置 chroot 导入验证 |

全量 Clippy 同时修正原有 fleet report 测试的无必要 clone，断言语义不变。

额外隔离运行验证：以准备好的 Python rootfs 启动真实 runc，挂载执行专属凭证目录；
容器内读回 fixture 成功，写入返回 EROFS，进程退出 0。未使用生产凭证或业务目录。

验收：在独立冻结检出 `d9b366a66dc7defa4281f484dd00b2a3e208c092` 加本轮工作区改动上，
workspace Clippy（all-targets，warnings denied）、build、全量 test 均通过；5525 passed、0 failed、7 ignored。
忽略项为已有需挂载/NFS、手工 runc 或浏览器环境的测试，不计为通过；另有真实 runc 私有目录只读验证。
冻结验证避免主工作区并行提交与共享构建产物替换造成版本混用；该结果不覆盖其后其他任务改动。

生产闭环预检使用独立源快照 `a533441dde8415dac98142f54a4fa92b9ae2e9af` 加本轮工作区改动，
Clippy、build 通过。全量测试首次有两个 runc 用例在高负载下超时，降低测试并发后完整重跑通过：
5535 passed、0 failed、7 ignored。发布包尚未生成，生产发布与运行态验收仍待执行。
