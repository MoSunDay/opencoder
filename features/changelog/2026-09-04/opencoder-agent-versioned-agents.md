Commit: 6cb5ea1

# 版本化自定义 Agent（共享资源池 + 引用卡）

## 主题

自定义 agent 从「每 agent 私有嵌套版本树」改为**共享池 + 引用卡**模型：`~/.opencoder/agents/` 下 `prompts|skills|tools|memory/<名>/v{n}` 是顶层共享版本化实体，agent 只是 `agents/<名>/meta.json` 引用卡（四字段引用资源名）——多 agent 共享同一份资源，资源升级（新版本）全员生效；版本号只增不复用，回滚=切 current 指针不删历史。active marker / 原子写 / preflight 回滚 / 静默降级全部复刻 envs 架构。

## 落点

- **core 读路径**（`crates/core/src/agent/{mod,meta,resource,compose}.rs`）：`resolve_agent` 文件 fallback（builtin 优先；卡片→prompt 资源 current 版本→`compose_prompt`（# Soul/# How/# Output + memory 追加 # Memory）；失败静默 None）；`effective_default_agent`（cli > active marker > `agent.default` > act）；skill 多根发现 `discover_all`（active agent 技能根在前，first-wins 遮蔽全局同名；多根指纹缓存）；`AgentDefaults` 增 `agents_dir/tools_scope/nfs`；`ToolContext` 增 `tools_path`。
- **opencoder-agents crate**（`crates/agents`，写路径 + NFS；初落为 `opencode-agents`，命名统一后更正，session 侧随之解耦——`tools_paths` 收口 core `agent::resource`）：`save_resource_version`（temp 目录→rename 原子落位）/`create_agent`/`update_agent_refs`（仅变更字段记 history）/`rollback_resource`/references 扫描快照；`nfs.rs`+`serve.rs`：nfsserve 0.11 只读 NFSv3 导出（filehandle=相对路径、一切写 NFS3ERR_ROFS、独立线程 + 优雅 shutdown、真实内核 mount 验证）。
- **session**：`/agent <名>` 任意 agent 热切换（AgentSwitch + persist + 刷新 tools_path/skill_roots；未知报 Error，裸命令列清单）；bash `-lc` 脚本**前缀**注入 `export PATH=...`（login shell 会重算 PATH，env 注入无效）；session skill 收口点按会话 agent 的 roots 遮蔽全局。
- **cli/tui**：fresh 默认链走 `effective_default_agent`；`TuiOpts.agent` 透传（main.rs → app_bootstrap 两条 fresh 路径）；resume 持久化文件 agent 名原样恢复。
- **web + SPA**：`/api/agents*`（卡片 CRUD/激活 preflight+回滚/共享池版本 CRUD/rollback/文件读取/被引用 409/仅生效链路变化 fan_out）+ `GET/POST /api/agents/nfs` 生命周期 + daemon 自启动；SPA「Agent 配置」（agentsConfig/agentDetail/promptEditor/agentNfsCard）。
- **与并行流的冲突消解**：`crates/agent`（opencode-agent 舰队 worker）已被占用，本特性写路径 crate 落在 `crates/agents`（opencoder-agents）。

## 测试清单

- core（`cargo test -p opencoder-core` 379 全绿）：meta marker/preflight 回滚/保留字（5）；compose 定序与稳定性（4）；resolve 文件 fallback/共享池升级传播/静默降级/默认链四级（~19）；skill 多根合并/遮蔽/缓存失效/根序（10）。
- opencoder-agents（27 绿 + 1 ignored 手动 mount e2e 已实跑通过）：版本递增不复用（回滚后再存跳号）、原子性（无 temp 残留）、history 仅变更字段、references 扫描、tools_paths 三分支、rollback 指针语义、NFS trait 层（lookup/read/readdir/ROFS/穿越拒绝）+ serve 生命周期 smoke。
- session（101 套件全绿）：`tests/agent_switch_file.rs`（切换事件/持久化/未知报错/复合命令/裸列清单）、`tests/agent_tools_path.rs`（login shell 下 `command -v` 命中池版本、展示纯净、ReloadConfig 后 v1→v2 重钉）、`tests/agent_skill_pools.rs`（私有遮蔽全局/切回恢复）、`tests/resume_file_agent.rs`。
- web（55 套件 ×3 轮全绿）：`tests/web_agents.rs`（激活 fan_out 恰一次/preflight 回滚保持旧 marker）、`tests/web_agent_resources.rs`（版本 1→2/rollback/b64 回程/1.5MiB 边界/409 引用保护/非生效链路静默）、`tests/web_agent_nfs.rs`（start/reuse/stop/失败路径）。
- cli/tui：`agent_override.rs`（文件 agent 过双接缝）、`parse_agent_name` 三态、`tests/bootstrap_agent_override.rs`（marker>cfg>act 层级）。
- SPA（vitest 326+ 全绿，新增 24）：`agentsItems/agentsConfig/agentDetail/agentNfsCard`；`npm run build` dist 固定文件名白名单不变。

## 全量回归结论

`cargo test --workspace --no-fail-fast`：293 套件 ok；仅 7 个失败套件全部位于 `opencoder-store` 迁移测试（brain/display_text/inputs_recorded/project_store/schema_bootstrap/schema_v4_migration/store_migrations），属并行流 store schema 工作树中间态，与本特性无依赖（store 测试零涉及 agents）。本特性触及的 core/agents/session/web/cli/tui 套件全绿；SPA vitest 327/328（唯一失败为并行流 brainPanel 超时抖动，隔离复跑通过）。
