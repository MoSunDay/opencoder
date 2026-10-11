Commit: 5c46834f223daf421f412b7cd6f104ec12defea6

# 后台管理表格与页头

用户权限、Token 管理和节点列表统一使用搜索、筛选与分页表格，创建和编辑由按钮打开表单对话框。用户权限与 Token 管理取消页头标题和说明，保留导航中的名称。表单提交、权限保护和失败处理见[后台管理表单](admin-forms-and-session-rounds.md)。

后台样式限定在 `oc-admin-` 前缀内，避免影响 Ontology。窄屏下操作列不固定，表格内容可横向滚动，弹窗适应视口。四种屏宽的验收选择器等待当前可见的菜单选项，避免关闭中的浮层干扰页面切换。

## 功能与测试

| 功能 | 测试或验收 | 文件 |
| --- | --- | --- |
| 用户和 Token 页面只保留导航标题 | `users keeps exactly one title source`、`tokens keeps exactly one title source` | [headerContract.dom.test.jsx](../../../crates/web/spa/src/shell/headerContract.dom.test.jsx) |
| 修改角色先确认，提交期间避免重复请求 | `saves a role only after confirmation and prevents duplicate submission` | [access.dom.test.jsx](../../../crates/web/spa/src/admin/access.dom.test.jsx) |
| Token 签发弹窗及明文仅展示一次 | `issues a token from a dialog and removes plaintext after acknowledgment` | 当时的 Token 页面交互测试 |
| 创建用户、签发 Token、变更角色、撤销及删除 | 隔离服务真实权限操作 | [access.js](../../../scripts/acceptance/ui/scenarios/access.js) |
| 四种屏宽下的页面切换和布局 | 全站响应式验收 | [probes.js](../../../scripts/acceptance/ui/responsive/probes.js) |

## 验证结果

- SPA：159 个测试文件、1,130 项测试通过；构建与产物一致性检查通过。
- 全站验收：16 项检查通过，覆盖 17 个注册页面、1920/1280/768/390 屏宽和 Server TUI。最终回执为 `/tmp/opencoder-admin-ui-final/receipt.json`；管理页与弹窗另有 36 项布局检查通过，回执为 `/tmp/opencoder-admin-ui/receipt.json`。
- Rust：workspace 构建、格式及 Clippy 检查通过。初次全量 Cargo 测试存在挂载权限与网络环境失败；在隔离的挂载、网络和登录配置环境中重跑同批编译的全部 437 个测试目标，5,747 项通过、零失败、8 项原有忽略。原 Cargo 运行的 21 个文档测试目标均无测试用例。最终回执为 `/tmp/opencoder-admin-isolated-rerun.json`，保留初次失败日志 `/tmp/opencoder-admin-rust-final-verified.log`。
- 发布相关 Python 回归：145 项通过。此项页面修改未执行发布。

实现索引：[Web](../../../agents/web/index.md)。页面行为见[Agent 平台](../../agent-platform/index.md)。
