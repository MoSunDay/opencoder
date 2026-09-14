Commit: 104c6b2663858f0a15d7066eb89227d681a4cf44

# Agent Harness — OpenCoder／Codex 执行方式与 Wrap 参数

## Agent 配置

- Agent 列表显示名称、生效状态与编辑／启动／删除操作。编辑从右侧打开占视口 75% 的详情抽屉，关闭后继续使用原列表。
- 执行方式、资源引用、Prompt 编辑和版本历史在 Agent 详情内维护。内置和自定义 Agent 都可以选择 OpenCoder 或 Codex。
- Codex Agent 可以绑定命名参数配置；清除绑定后使用默认 Codex 配置。参数配置读取失败时显示错误并提供重试，期间禁止修改绑定。
- 宿主机执行使用 Operator 入口，不提供单独的 Runner 配置页。

## Codex 参数管理

- Harness 管理只配置 `opencoder --wrap codex` 的 `--model` 与 `--envs`，支持默认配置和命名配置档案。
- 模型留空时使用 Codex 默认模型。环境变量每行一个 `KEY=VALUE`，允许空值，保留值中的空格与等号；格式错误时禁止提交。
- Web 不提供 Codex 安装路径、授权槽位、推理强度、沙箱或审批策略控件。保存时仅提交模型与环境变量，其余 Codex 选项回到默认值，由执行节点上的 Codex 自身配置决定。
- 读取失败时禁止用空配置覆盖现有值；保存失败时保留输入并显示错误，允许重试。

## 执行边界

- Codex 使用节点自身认证与权限，不使用 OpenCoder 凭据。
- 配置在任务受理时固定；修改只影响后续新受理任务，已排队或运行中的任务保留原配置快照。
- 环境变量保存在私有定义库，不进入 NFS 资源，公开执行详情不返回其值。
- 意外退出不自动重发已提交需求。

## 相关

- [agents/session](../../agents/session/index.md)
- [agents/local](../../agents/local/index.md)
- [agents/web](../../agents/web/index.md)
- [agents/worker](../../agents/worker/index.md)
