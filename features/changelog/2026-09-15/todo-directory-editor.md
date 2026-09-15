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

- 代码候选 `d1779dd844bc5f61496cb3be5978bc6217b71cb4` 合并 TODO 目录、平滑发布和线上 `5bf6f621` 的全部 DAG 修复。发布分支为 `release/todo-directory-20260915`。
- 全量 Rust 回归结合失败项及新增用例复验，共覆盖 5,228 项通过；6 项既有特权手工用例保留忽略。原全量日志 `/tmp/opencoder-todo-delivery-workspace-regression.log`，修复后的 205 项复验日志 `/tmp/opencoder-todo-delivery-regression-fixes.log`。
- `cargo clippy --workspace --all-targets -- -D warnings` 零警告；前端全量 96 文件、683 项通过；发布工具及备份安装测试 33 项、真实模型验收脚本测试 3 项通过。
- 成套优化发布包的四个二进制提交号、协议与静态产物摘要一致。浏览器 TODO 与 DAG 验收通过；真实磁盘的三版本切换、带任务回滚、Shell/OCI 进程保持、SSE 连续及独立 NFS 验收通过，证据 `/var/tmp/opencoder-smooth-h7quack9/result.json`。
- 联合发布已切到 `320dbbf3`；完整 Rust 回归 5,228 通过、0 失败，生产 TODO 依赖链、目录 Review、跨版本执行和 900 秒稳定性观察通过，详见 [平滑发布验收](smooth-release.md#生产迁移与验收)。
