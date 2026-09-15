Commit: 1ac64fe8b81a2c7c144c72b717a8031ab18f2589

# TODO 目录编辑与文件校验

## 交付行为

- TODO 模板使用运行时可加载的真实目录：工作流配置和环境绑定为 JSON，目标、任务背景、执行要求及验收标准为 Markdown。
- 目录编辑器支持 JSON 高亮与格式化、Markdown 预览与分屏、文件搜索、独立撤销历史和保存快捷键。任务新增、复制、改名与删除同步维护目录和依赖。
- 不合规文件在加载、校验、保存和运行前弹窗报错，显示路径、行列和原因，可定位修复。缺失文件可补写，多余文件可移除；无效定义不会发布或执行。
- 保存创建不可变的新版本并切换当前版本，修订检查拒绝并发覆盖。旧 context.json 模板保持可读；CLI 修改通过新版本接口发布。
- 运行 Review 使用只读定义和过程目录，每次派发分别保留上下文、候选结果及会话。指定任务重跑保留旧历史和独立分支；父 Agent 会话查看不清空当前任务选择。
- 移除注册业务 Runner 的配置类型、执行模块、管理接口及专用详情逻辑。DAG 列表移除类型和步骤数量，保留名称、说明、更新时间和操作。

## 验证

- 全量前端：96 个测试文件、683 项通过。
- TODO/DAG：130 项；CLI 与配置：27 项；Control TODO 接口：26 项；Web 模板与运行：9 项；Node Review 与 Runner 拒绝：3 项。
- 真实 Server、Node 与 Chromium，使用受控模型回执验证目录创建、JSON/Markdown 编辑、错误弹窗、版本保存、实际定义目录、只读 Review、重跑、离线恢复、节点重启和窄屏布局。
- SPA 构建产物与源码一致；Git diff 格式检查通过。

相关：[TODO 工作台](../../../docs/todo-workbench.md)、[功能说明](../../todos/index.md)。

## 联合发布验证

- 生产版本 `320dbbf3a5b8e843c5f2650e09059a7d8cf41b70` 已生效，合并 TODO 目录、平滑发布和线上 `5bf6f621` 的 DAG 修复。后续源码提交补充发布工具的本机挂载检查与首次迁移处理，业务二进制源码一致。
- Rust 全量回归 5,228 项通过、0 失败，6 项既有特权手工用例保留忽略；最终完整日志 `/tmp/opencoder-todo-delivery-workspace-final.log`。全目标 Clippy 零警告，workspace 构建通过。
- 前端全量 96 文件、683 项通过；发布工具 16 项、安装备份 19 项、真实模型验收夹具 3 项通过。四个优化二进制的提交号、协议和静态产物摘要一致。
- 隔离的真实磁盘演练覆盖三版本切换、带任务回滚、Shell/OCI 进程保持、SSE 连续及独立 NFS，证据 `/var/tmp/opencoder-smooth-h7quack9/result.json`。

## 生产验收

- 线上 DAG 列表不显示类型和步骤数量；编辑器只提供 Agent、Wasm；已删除 Runner API，Runner 定义请求返回 400。
- 实际浏览器完成目录创建、JSON/Markdown 编辑与预览、保存新版本、旧版本保持不变。JSON 语法错误和空 Markdown 均弹窗显示路径、行列和原因，禁止创建或发布无效版本。
- 真实模型执行两个依赖 TODO，核验 Runtime 加载冻结目录，第二个任务获得首个任务的已验收结果。只读 Review 和指定任务重跑通过，上游文件与会话不变，目标任务新增会话且保留历史。
- 发布期间 33 次连续提交零失败；旧 Runtime 与工具进程保持，新任务进入新版。独立进程完成完整 900 秒观察，173 个样本全部通过。首次观察进程收到 SIGTERM 后保留了部分记录，最终通过基于重新完成的完整窗口。
- 发布状态为 complete；Server、Host、Runtime、Nginx 与资源服务均正常，检查时无异常重启和错误日志。原 Node ID、凭证及并发上限 20 保留；一致性备份已完成。
- 结果与截图：`/var/lib/opencoder-platform/acceptance/todo-directory-20260915/release.json`。跨版本与观察证据：`/var/tmp/release-live-68cd9f401086e095/result.json`。详见 [平滑发布记录](smooth-release.md#生产迁移与验收)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 完整目录往返与 Markdown 保真 | `directory_round_trip_preserves_all_spec_fields_and_markdown_bytes` | `crates/todos/tests/directory.rs` |
| 逐文件校验与错误位置 | `validates_every_required_file_and_returns_file_locations` | 同上 |
| 不可变版本与旧模板读取 | `atomic_versions_and_legacy_import_keep_the_source_unchanged` | 同上 |
| 无效 JSON 保留草稿、弹窗定位、阻止保存 | `preserves invalid JSON while switching files and blocks save with a locating modal` | `crates/web/spa/src/todoEditor.dom.test.jsx` |
| Markdown 源码与安全预览 | `Markdown supports source and sanitized preview without changing saved text` | 同上 |
| 只读历史选择与实时刷新 | `loads older attempts and retains the selected context when live history refreshes` | `crates/web/spa/src/todo/review/files/workspace.dom.test.jsx` |
| 重跑回执丢失后幂等恢复 | `keeps one request identity across a lost response and closes only after durable queuing` | `crates/web/spa/src/todo/review/rerun.dom.test.jsx` |
