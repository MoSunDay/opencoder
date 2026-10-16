Commit: 1ba83e9ec97a38106af1bbe309860a3e5276cc4f

# 鉴权开关与项目 CLI / Skill

## 需求与范围

用户要求为鉴权增加开关，将 10.199.79.43 设为关闭，并提供项目、专项、TODO 的 CLI 与 Skill。复用现有项目 API 和 CLI，不修改用户或 Token 数据，不改看板、归属、删除保护或执行规则。本次发布包含已提交的项目功能和本条改动；此前未发布的执行输入、文件交接改动保留在主工作区，另行发布。

## 实现

- `opencoder-server --auth-enabled true|false` 默认开启，部署声明为布尔 `deployment.auth_enabled`，新 Server unit 传入关闭选项。关闭时跳过启动管理员凭据写入；Host、Runtime 和独立资源服务继续使用现有机器凭据。
- 公共 `GET /api/auth/status` 只返回当前实例开关。关闭时请求使用固定 admin 身份，网页直接进入控制台；原 Token 不影响角色，重新开启时恢复校验与角色权限。
- `opencoder-cli --no-auth` 或连接文件 `auth_enabled:false` 省略 Authorization，且不读取 Token 文件。默认仍要求凭据，显式 Token 参数覆盖连接文件设置；CLI 输出与退出码不变。
- 复用 `project goals/initiatives/todos list/get/create/patch/delete`，新增可安装 `opencoder-projects` Skill，说明归属、人员、标签、评论、概述、失败处理与删除保护。
- 本机实际访问内网被已有代理返回 502，设置目标 IP 的 NO_PROXY 后返回 200；补充 `opencoder-79` 快捷命令，自动选择独立连接文件、43 地址及本进程代理绕过名单。通过真实 Server time、免鉴权状态与项目列表核对；该脚本随 Skill 交付，不改变发布二进制。

## 功能与测试

| 行为 | 实际测试 |
| --- | --- |
| 默认开启、显式布尔值、非法参数拒绝 | Server `authentication_defaults_on_and_requires_an_explicit_boolean` |
| 配置与 unit、生效参数、保留机器凭据 | Python `AuthenticationConfigTest.test_explicit_boolean_controls_the_server_without_changing_credentials` |
| 免 Token 三层完整 CRUD、JSON 文件、评论署名、删除保护、原用户与 Token 保留 | CLI `disabled_auth_supports_all_project_crud_and_preserves_existing_credentials` |
| 显式免鉴权与凭据互斥 | CLI `anonymous_flags_are_explicit_and_conflict_with_credentials` |
| 原身份与角色门禁、独立指标凭据 | Control `users_api::*`、`scheduler_metrics::*`；Web `auth_mw::*` |
| 免鉴权资源代理仍使用机器凭据、资源服务保持保护 | Control `resource_service_publishes_native_versions_to_its_exported_pool` |
| 原 CLI Token 默认连接、项目调用、概述读取 | CLI `server_local` 的 default_file、system_probes、project_catalog 定向用例及 `project_summary` |
| 免 Token 打开控制台、保持原登录和退出、导航 | SPA `app.dom.test.jsx`、`appNavigation.dom.test.jsx`、`navPersistence.dom.test.jsx`、`sidebar.dom.test.jsx`、`login.dom.test.jsx` |
| 免 Token 请求与产物下载 | SPA `api.test.js`、`fleet/download.test.js` |
| Skill 命名、说明、引用与字段 | `skill-creator/scripts/quick_validate.py` 及 CLI 实际 CRUD |

本轮只运行上述改动和直接影响链路的定向测试、相关四个 crate 的 Clippy；不运行 workspace 全量测试、SPA 全量测试或全站 UI 验收。源码、包和生产结果分别保留，不将局部通过表述为全量通过。

本地证据目录 `/tmp/opencoder-auth-project-20261014`；43 上的发布、配置原件、鉴权表摘要、CRUD 与 900 秒观察回执保存在 `/data00/opencoder-delivery/auth-project-20261014`。实际生效版本和结果以最终回执为准。

## 上线结果与验收范围

- 正式代码与七程序发布包为 `9a6123bc62f1019f12a046680890e35dd1b69099`。Signal 发布已完成，公共入口实际返回 `enabled:false`、匿名 admin，机器凭据保留；七个部署文件的摘要和完整构建信息一致，Server、Host、Runtime 为 active。
- Rust 定向 61 项、SPA 定向 52 项、发布配置与信号脚本 43 项通过，正式快照上的新鉴权集成用例和四个相关 crate 的 Clippy 通过；SPA 成套构建与漂移检查通过。未运行全量回归。
- 43 的原生 CLI 无 Token 完成 24 次请求，包括三层完整 CRUD、归属过滤、文件 JSON、评论、删除保护及删除后 404；只清理此次创建的三个对象。桌面 1440 和手机 390 页面均免 Token 进入，无页面异常。
- 鉴权库仅只读核对，原有一个用户和一个 Token 的全表摘要前后一致。发布前承载任务的原 Runtime 与独立资源服务 PID/start ticks 保持。独立检查初版错误地要求所有闲置历史 Runtime PID 不变，原失败记录保留；已另存历史休眠/唤醒状态，与活跃任务连续性区分。
- 通用真实模型 / native 发布脚本已完成原 Runtime 长任务跨切换、两个依赖脚本、持续受理及 SSE 游标校验。切换受理与 readiness 零失败；最大接收 14.355 秒、P95 8.406 秒、最大调度间隔 21.476 秒，ready 最大间隔 1.403 秒。
- **通用 native 900 秒观察未通过**：完成 64 次检查后，第 65 个探针 `dag-release-live-e02baaeba7d24307-observe-64` 超过 30 秒等待门槛。实际开始延迟 32.208 秒，随后成功；当时节点上有 18/20 个任务在运行。未提高门槛、抢占其他任务或将失败改写为通过，`failure.txt` 与原 unit 的失败状态保留。
- 本次鉴权与项目接口独立连续观察 900.0 秒通过，1074 次公共 API 请求零失败，最大单请求 4.343 秒、最大检查间隔 9.356 秒。回执为 `auth-observation.json`；它只读取六个公共 API、不创建执行，不能代替上述未通过的通用 native 观察。

## 用法

[项目 CLI 与鉴权](../../../docs/project-cli.md)、[opencoder-projects](../../../skills/opencoder-projects/SKILL.md)、[ctl](../../../agents/ctl/index.md)、[Server](../../../agents/server/index.md)、[Control](../../../agents/control/index.md)、[Web](../../../agents/web/index.md)。

观察期间入口更新到 `21d21a6bf3274d74d7d764b6f0b86d001c07bc03`，包含 `9a6123bc` 的功能以及 Codex 容器启动修复与外部连接验证，鉴权开关仍关闭。七个程序摘要与构建信息、服务状态、鉴权表摘要再次核对通过。新版单次匿名 native 执行补查 0.395 秒通过，原 900 秒 native 观察仍记为未通过。
