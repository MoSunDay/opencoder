Commit: e3a3dddaf98f81743fb6880b6972393df72733e7

# 创建用户自动签发 Token

创建用户同时生成一个可立即使用的 Token，管理员无需再进入独立签发页面。创建响应仅返回一次明文并禁止缓存，数据库只保存摘要；用户和初始 Token 共用已有事务，不增加表或环境变量。

用户权限页在创建成功后展示可复制的 Token，确认保存后清除；刷新或重新进入页面不能找回。角色编辑不签发 Token。移除独立 Token 管理页及导航，保留管理 API；未操作已有环境的用户或鉴权数据。

## 功能与测试

| 功能 | 测试 |
| --- | --- |
| 创建即用、仅一次返回、重复创建和角色变更不增加 Token、删除后失效 | [users_api.rs](../../../crates/control/tests/e2e/users_api.rs)：`user_creation_issues_one_usable_token_without_later_disclosure` |
| 初始 Token 保存失败时回滚用户 | [access_tokens.rs](../../../crates/store/tests/access_tokens.rs)：`initial_token_failure_rolls_back_user_creation` |
| 一次展示与管理员保护 | [access.dom.test.jsx](../../../crates/web/spa/src/admin/access.dom.test.jsx)：`creates a user and shows its token once while protecting the startup administrator` |
| 重复提交、离开后清除、列表刷新失败仍保留新 Token | 同文件：`prevents duplicate creation and forgets the token when leaving the page`、`keeps the issued token visible if the following user list refresh fails` |
| 创建失败保留草稿、编辑角色不签发 Token | 同文件：`keeps failed creation inside the dialog with the draft available for retry`、`saves a role only after confirmation and prevents duplicate submission` |
| 真实浏览器复制、刷新后隐藏、登录鉴权与四种屏宽 | [access.js](../../../scripts/acceptance/ui/scenarios/access.js) |

## 验证

- 开发回归基于 `42456123` 加本任务改动的独立工作区，避免混入共享工作区中其他任务的并行改动；SPA 159 个文件、1,130 项通过，Rust 5,749 项通过、0 失败、8 个原有 ignored。
- 格式检查、Clippy 零警告、workspace 与 examples 构建、SPA 构建及产物一致性检查通过；发布脚本 66 项、验收脚本 9 项通过。
- 全站 UI 验收覆盖 16 个页面及 1920、1280、768、390 四种屏宽，原生执行和 TUI 均通过；另运行原生 Brain 浏览器测试并通过。回执：`/tmp/opencoder-user-token-20261010/global-ui/receipt.json`。
- 对共享工作区当前 SPA 产物另行运行用户创建、复制、四种屏宽、刷新隐藏、登录、角色修改、删除失效流程，全部通过。回执：`/tmp/opencoder-user-token-20261010/access-current-Bbsm8Q/receipt.json`。
- 开发回归日志保留于 `/tmp/opencoder-user-token-20261010/`。

## 发布

- 2026-10-10 通过当前 Server 信号发布 `rel-e3a3dddaf98f81743fb6880b6972393df72733e7`，发布回执为 `complete`。独立工作区只包含本功能及所需接口改动；本地分支为 `release/user-initial-token-20261010`。
- 发布源码与此前完整回归的代码快照按文件摘要一致；正式包另行完成全站 16 页、四种屏宽、原生执行和 TUI 验收。目录准备及浏览器检查中的失败记录保留，修正验证设置后以同一发布包继续验证。
- 线上只读浏览器检查确认文件摘要一致、无 Token 管理入口、创建提示和弹窗正常；没有创建、修改或删除线上用户与凭据。15 分钟健康观察完成，178 次检查全部成功，4 个节点保持就绪。
- 任务连续性专项未计为通过：首次等待发布回执 300 秒超时，但独立发布作业继续完成；第二次等待原生测试任务进入运行步骤 90 秒超时。两轮测试创建的四个执行最终均为 `done`，相关失败回执保留；线上健康观察不替代该专项验收。
- 发布和验证回执汇总：`/var/tmp/opencoder-user-token-publication-20261010/delivery.json`。正式 UI 回执为同目录 `release-ui-final/receipt.json`，线上截图为 `public-browser-stable/`，健康观察为 `active-verification.json`；临时目录中的 UI 证据归档为 `ui-resume-evidence.tar.gz`。

## 相关索引

- [平台功能](../../agent-platform/index.md)
- [Control 模块](../../../agents/control/index.md)
- [Web 模块](../../../agents/web/index.md)
