Commit: e50ffc433bca866fd17bd571a74f1bdf17705dea

# 删除节点 ENV 配置集体系，强化 TODO env 运行时

## Context

仓库曾有两套独立 ENV：节点侧 OpenCoder Env 配置集（`/api/envs`，NFS `envs/<name>/` 下的 config/mcp/cli/skills/ap.json，激活后插入配置解析层）与 TODO env（`/api/todo/envs`，`env/<name>/context.json`）。前者与核心配置层的 env 遮蔽链、worker 资源 pin、TUI /envs 菜单、SPA Env 管理页强耦合，语义与 TODO env 重叠且维护成本高；后者只是声明性记录，`env_vars` 没有任何运行时消费者。

## Change Summary

- 整体删除节点 ENV 体系：core `config/envs.rs` 与配置候选链 env 层（`config_candidates` 收敛为 project-first）、control `/api/envs` 三条路由与 `api_envs.rs`、web `api_envs.rs` 与测试、worker 资源快照的 `envs` pin 特判、TUI `envs_menu/` 与 `/envs` 斜杠命令、SPA `envsPanel` 与导航入口、local `session_cmd` 的激活 banner。
- TODO env 成为唯一"环境"体系，`env_vars` 真正生效：
  - dispatch 盖章：control `api/template.rs` 与 web `api_todo_runs.rs` 把绑定 env 的 `env_vars`（键排序、键必须匹配 `looks_like_env_var`、值必须字符串，fail-fast）写进 spec 快照 `metadata.env_vars`；web 保存 env 时同步校验。
  - 运行时注入：`todos/src/execution.rs` 在子会话启动后把 `workflow.metadata.env_vars` 并入 `session.env_passthrough`（BTreeMap 合并，TODO env 覆盖 resume 恢复的 harness env），经 `ToolContext::extra_env` 抵达 bash 工具进程与 Codex harness 进程。
  - 生效证明：`crates/todos/tests/env_passthrough.rs` 用 MockChatClient + bash 轮次断言子会话 bash 输出携带 env 值；web/control e2e 断言快照盖章与 env_vars 传递。

## Impact Surface

- core：`config.rs`/`config/env.rs`/`config/domain.rs`/`lib.rs`、`agent/meta.rs`
- control：`routes.rs`、`resource_scope.rs`、`api/template.rs`
- web：`lib.rs`、`api_todo_runs.rs`、`api_todo_envs.rs`、SPA 导航
- worker：`resources.rs`（保留 `input.envs` harness per-task env，属另一体系）
- tui/local：斜杠命令与 banner 清理
- todos：`domain.rs` 三件套 helper、`execution.rs` 注入点、新增 `tests/env_passthrough.rs`

## Notes / Compatibility

- 已部署的 spec 快照若无 `metadata.env_vars` 字段，运行时按空处理（向后兼容）；手工编辑的畸形条目在运行时被跳过不崩溃。
- `/api/todo/envs` 及 SPA `src/envs/` TODO 模板环境入口保留并承担全部环境职责。
