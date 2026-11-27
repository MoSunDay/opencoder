# Shared/Act/Plan 模型配置与本地 Env

一句话：模型与生成参数支持 Shared/Act/Plan 分层覆盖，TUI 提供分模式保存和
会话级选择；本地命名 Env 可集中切换配置、CLI 与 Skill 文件，同时保留 legacy
配置、会话恢复和隔离加载语义。

## 已实现行为

### 配置解析与请求

- [mode.rs](../../../crates/core/src/config/runtime/mode.rs) 的
  `Config::effective_for_mode` 按 legacy 顶层字段 → `modes.shared` → 独立的
  `modes.act` 或 `modes.plan` 解析 `model`、`reasoning_effort`、
  `interleaved_thinking`、`max_tokens`、`context_limit`、`cache_salt`。
  缺少或为空 `modes` 时保留完全相同的 legacy 配置视图；补丁中的显式 JSON
  `null` 删除对应覆盖并恢复继承。
- 只有精确匹配的内置主 `act`/`plan` agent 使用 Act/Plan 视图；自定义 agent、
  build/subagent/workflow 与辅助会话继续使用 Shared。每次请求使用完整的有效
  `provider/model`，`ChatClient::prepare` 负责 provider 端点重路由并移除 wire
  模型名前缀；有效生成参数与当前模式上下文限制同时供请求、压缩和硬限制检查。
- 进程级模型优先级为 headless `--model` > `OPENCODER_MODEL` > resume 恢复的
  `sessions.model` 与 Act/Plan 会话选择。`preferred_explicit_model` 只保留前两者中的
  胜者，resume 后重应用同一胜者，避免环境值反超 CLI 或恢复值反超显式选择。
  `Config::apply_explicit_model` 统一设置顶层模型并仅清除各模式的 `model` 覆盖；
  `reasoning_effort`、`interleaved_thinking`、`max_tokens`、`context_limit` 与
  `cache_salt` 等生成参数和 context overlay 均保留。
- 已持久化的模式感知会话应用显式模型时写入空元数据 `{}`，保持 Act/Plan 继承
  显式 Shared；legacy 行仍为 `NULL`，不会被无谓转换。新会话仍延迟到首条输入才落行。
  已存在会话先更新 `sessions.model` 再写模式元数据；任一步失败都会向上传播，第二步
  失败会补偿第一步，显式模型路径还恢复内存中的 config、模型标签和请求身份，避免
  内存新值/磁盘旧值分裂。

### TUI 保存与会话选择

- `/model` 和 `/config` 先选择 Shared/Act/Plan。Shared 保留原顶层 `/model`、
  `/config` 行为；Act/Plan 模型写入 `modes.act.model`/`modes.plan.model`，生成
  参数写入对应模式块，`i`/Delete 用显式 `null` 恢复继承。
- Provider 新增、编辑、删除统一进入保存范围确认：`y`/`Y` 或 Enter 写磁盘，
  `n`/`N` 只应用到当前会话，Esc 取消本次变更且不切换模型。菜单显示
  `Active Env` 和已解析的 `Save target`。
- 模型菜单的 List/Form/ModeConfig 与 `/cli`、`/skill` 显式使用
  `ConfigSaveRoute::EnvAware`：有活动 Env 时，模型、分模式生成参数、CLI、Skill
  分别保存到其 `config.json`、`cli.json`、`skills.json`；无活动 Env 时回退 `Auto`，
  沿用既有可编辑项目/全局目标识别，最终回退 `<workdir>/opencoder.json`。
- `Auto`（含 `Config::save`）和显式 `Project`/`Global` 不会因活动 Env 改道，
  `ActiveEnv` 则要求 Env 已激活，否则报错。领域保存同样按显式 route 分流，只有
  CLI/Skill 支持活动 Env，MCP 与 schedules 不会被隐式写入 Env 文件。
- Shared 的“仅会话”更新 `sessions.model`；Act/Plan 更新独立的会话模型覆盖
  元数据，不写配置文件。模型选择会随 resume/fork 保留，并非仅存在当前进程。
- session-only Provider patch 支持新增、编辑及以 `null` 删除 Provider；显式
  `api_key: null` 清空 key，`headers` 整体替换而非追加。变更只合并进候选内存
  `Config`，不写磁盘；Provider 实际变化时由 `ApplySession` 按新的 endpoint、API key
  与 headers 重建 worker client。它们不属于可随会话恢复的模型元数据。
- 单独切换模型只发送会话选择，不重建 client；后续请求仍由
  `ChatClient::prepare` 按完整 `provider/model` 重路由。session-only 模型/Provider
  通过带 ack 的 `ApplySession`，Env 切换通过带 ack 的 `ReloadEnv`：worker 先完成
  client 构造、会话持久化和运行时应用，再返回成功；UI 仅在收到 `Ok` ack 后提交
  外层 config/client/chat/context 状态并显示成功标记。发送失败、worker 结束或
  Store/client 错误均保留原 UI 运行时，只显示错误，不显示成功。

### 本地 Env 与重载

- Env 位于 `~/.opencoder/envs/<name>/`，包含 `config.json`、`cli.json`、
  `skills.json`；活动选择记录在 `~/.opencoder/active_env`。新建 Env 将三个
  配置文件初始化为 `{}`。
- `/env` 与 `/envs` 打开管理菜单；命令支持 `/env [name]`、`use`/`activate`、
  `create`/`new`、`delete`/`remove`/`rm`、`off`/`none`/`deactivate`。菜单键为
  Enter 使用、`n` 新建、`o` 不使用 Env、`d` 删除。
- `config.json` 候选合并中活动 Env 高于项目、全局和 XDG。CLI/Skill 等领域文件
  按活动 Env → 项目 → 全局 → XDG 选择一个有效文件，不做跨文件逐 key 合并。
- Env 切换通过 `ReloadEnv` 清除 Act/Plan 会话覆盖，重载配置/client 与后续请求使用的
  CLI/Skill 状态，并同步 MCP pool；`skill_body_delivered` 保留，因此已消费的一次性
  Skill body 不会因切换再次投递。已落行会话同时把目标 Env 的 Shared 模型持久化到
  `sessions.model`，并写入空模式元数据 `{}`，所以 resume 不会恢复旧 Env 的 Shared；
  新会话仍在首次落行时一起物化这两个值。Env 与显式模型共用持久化错误传播和
  `sessions.model` 补偿路径。
- 默认 `Config::load` 的所有现有调用方都会读取 active Env，并非只有交互式缓存加载。
  `load_with_home(..., Some(home))`、frozen 与 operator 加载保持隔离，不读取该交互
  状态；frozen 还跳过进程环境覆盖，operator 只读取显式目录。
- active marker 的非 `NotFound` I/O 错误会让默认加载及 `EnvAware` 目标查询/保存
  fail closed，不能静默改写项目或全局文件；可读取但指向无效/缺失 Env 的 stale marker
  仍被忽略，加载和 `EnvAware` 保存回退 legacy 候选。
- Env 激活 preflight 与正常合并一致地接受 `modes: null`：它表示清除继承的模式块，
  仅在独立 `Config` 解码前归一化为 `{}`；其他畸形字段仍会拒绝激活，marker 保持不变。

### 会话元数据、事件与 Worker 缓存

- Store schema v42 增加可空 `sessions.model_overrides_json`：`NULL` 表示 legacy
  单模型行为，`{}` 表示模式感知且 Act/Plan 均继承配置，新会话使用
  `Some(SessionModelOverrides::default())`。legacy 行转换时会为另一模式显式保留
  原 legacy 模型，不追溯套用新模式覆盖。
- 该元数据不属于既有 `SessionMeta` DTO，经
  `create_session_with_model_overrides`、`get_session_model_overrides`、
  `set_session_model_overrides` 访问；旧 bundle 缺少字段时使用默认
  `SessionBundle.model_overrides`。TUI `TsMirrorStore` 对三个接口原样委托 inner Store，
  保留 `NULL`/`{}`/显式值，metadata-aware 创建也保留既有 ts 镜像 hook。
- `SessionState::take_model_switch` 以完整有效 `provider/model` 对比 `active_model`：
  相同模型重复调用及仅生成参数变化不发送事件，provider 改变但裸模型 ID 相同仍会
  发送一次 `ModelSwitch`。它在 `LlmRoundStart` 前同步请求身份，并与 reload/set 操作
  对 `active_model` 的更新配合，避免 worker 即时事件和下一请求重复发送。
- `/task` New 继承当前 Shared 模型、Act/Plan 会话覆盖和当前完整 `providers`（含
  session-only 新增/编辑/删除），其他设置重新从磁盘加载。新 client 必须基于同一个
  `new_config` 构造，不能克隆旧任务携带的 provider 路由快照；继承先只写入新
  `SessionState` 内存，首条输入创建 session 行时才持久化，不提前产生空任务。
- 普通 `ReloadConfig` 在 Shared 模型变化时先持久化 `sessions.model`，失败则保留原
  worker config/client、发出可见 Error 且不发 `ModelSwitch`。`/cli`、`/skill` 的
  `ReloadConfig` 投递失败也不会提交外层 live config；错误标记明确说明磁盘已保存但
  reload 投递失败，不显示未限定的成功结果。
- Worker 配置缓存键覆盖每个文件的路径/size/mtime、active marker 与 stale 状态、
  活动 Env 文件及 workdir/global/XDG 发现身份，避免跨 Env 或跨发现目录误复用。
  无活动 Env、仅由 XDG 提供 `config.json`/`cli.json`/`skills.json` 时，三类文件的
  创建、同长编辑和删除同样改变指纹并刷新缓存。

## Ontology 能力目录一致性

原生[平台能力清单](../../../crates/core/src/ontology/platform.rs) 已在既有
`platform:tui` 实体登记六个 TUI 命令 `/env`、`/envs`、`/model`、`/config`、`/cli`、
`/skill`：说明包含 `/model` Provider 新增、编辑、删除的 session-only 选择，以及
`/cli`、`/skill` 使用 `EnvAware` 在活动 Env 与 legacy `Auto` 间选择保存目标。
`platform_capabilities()` 继续生成同一实体，`cli` 字段以 `TUI` 前缀区分交互式斜杠
命令。它们复用 `/api/tui` 的既有能力查询，不新增公共 HTTP 路由或 `opencoder-cli`
命令域，也不另建重复目录实体。

按规则 09 使用临时夹具执行隔离等价同步检查，而非连接生产或真实用户数据；本次
复审实际执行两个定向用例：

```text
cargo test -p opencoder-control --test ontology_catalog platform_tui_entry_synchronizes_interactive_env_and_mode_model_configuration -- --exact
→ 1 passed / 0 failed / 9 filtered out
cargo test -p opencoder-control --test ontology_catalog native_changes_automatically_update_catalog_and_keep_read_permissions -- --exact
→ 1 passed / 0 failed / 9 filtered out
```

前者验证六个 TUI 命令、Provider session-only 与 `EnvAware` 登记及只读同步检查；
后者执行原生目录同步，并验证 Viewer 对 `GET /api/ontology/envs/debug/sync/check`
的只读权限。两者都不是对在线 Server 执行 `ontology sync check`。

## 非范围

- 不扩展既有 TODO Env 管理，也未新增通用进程环境变量管理器。
- 未新增 MCP、schedule、Web/NFS 管理功能。
- 未新增 HTTP/API 或公共 CLI 入口，未修改无关 acceptance 脚本。
- 本次复审只补齐 Ontology 元数据、定向断言和文档，没有修改上述行为的运行时实现。

## 测试覆盖

| 功能 | 测试目标/检查 | 文件 |
|------|---------------|------|
| 模式继承、默认/隔离加载、marker 与 preflight | `explicit_model_overrides_every_mode_without_clearing_generation_fields`、`scoped_frozen_and_operator_loaders_ignore_interactive_active_env`、`unreadable_marker_fails_closed_while_stale_marker_keeps_save_fallback`、`null_modes_preflight_resets_merged_modes_without_accepting_invalid_config`、`env_aware_and_legacy_save_routes_do_not_leak_into_each_other` | [config_modes.rs](../../../crates/core/tests/config_modes.rs)、[config_local_env.rs](../../../crates/core/tests/config_local_env.rs)、[config_local_env_loading.rs](../../../crates/core/tests/config_local_env_loading.rs) |
| CLI/Env 显式优先级、`{}`/`NULL` 与持久化补偿 | `process_model_selection_prefers_cli_and_ignores_empty_environment`、`resume_reapplies_environment_or_cli_winner`；`persisted_explicit_model_writes_empty_object_or_legacy_null`、`lazy_env_reload_materializes_shared_and_empty_metadata_together`、`failed_model_metadata_save_rolls_back_explicit_and_env_changes`、`failed_override_save_rolls_back_memory_and_keeps_old_row` | [model_override.rs](../../../crates/local/src/model_override.rs)、[explicit_env.rs](../../../crates/session/tests/mode_model_persistence/explicit_env.rs)、[mode_model_persistence.rs](../../../crates/session/tests/mode_model_persistence.rs) |
| 请求路由、生成参数、恢复与事件去重 | `mode_model_requests`、`overrides_survive_resume_and_normal_reload_without_config_writes`、`inheritance_and_fork_copy_per_mode_selections`、`reload_config_compares_the_current_agents_full_effective_model`；migration/bundle/resume/fork/compaction 定向用例 | [mode_model_requests.rs](../../../crates/session/tests/mode_model_requests.rs)、[mode_model_persistence.rs](../../../crates/session/tests/mode_model_persistence.rs)、[tests_reload.rs](../../../crates/tui/src/worker/tests_reload.rs) |
| 可空模式元数据、bundle 与 TsMirror 委托 | `session_model_overrides`；`model_metadata_delegates_all_states_and_keeps_ts_mirroring` | [session_model_overrides.rs](../../../crates/store/tests/session_model_overrides.rs)、[ts_mirror.rs](../../../crates/tui/src/ts_mirror.rs) |
| Provider CRUD 保存范围与 ApplySession/ReloadEnv ack | `add_form_n_returns_session_only_patch_and_enter_preserves_disk_save`、`edit_api_key_and_headers_reach_session_only_outcome_through_form_keys`、`delete_n_is_session_only_while_y_saves_and_esc_cancels`；`provider_add_replaces_fields_and_rebuilds_only_after_ack`、`send_failure_and_worker_failure_leave_all_outer_state_unchanged`、`env_send_and_worker_failures_leave_outer_runtime_unchanged`、`session_store_failure_acknowledges_error_without_worker_mutation`、`env_override_store_failure_keeps_worker_and_client_unchanged` | [session_only_ui_tests.rs](../../../crates/tui/src/model_menu/tests/session_only_ui_tests.rs)、[model_session_switch_tests.rs](../../../crates/tui/src/model_session_switch_tests.rs)、[env_model_failure_tests.rs](../../../crates/tui/src/app_loop_tests/env_model_failure_tests.rs)、[tests_failures.rs](../../../crates/tui/src/worker/tests_failures.rs) |
| `/task` New 继承与基于 `new_config` 的 client | `new_task_keeps_current_shared_model_providers_and_other_settings`、`new_task_client_routes_with_added_edited_and_deleted_providers`、`resume_then_new_preserves_shared_for_legacy_and_mode_aware_sessions` | [app_task/tests.rs](../../../crates/tui/src/app_task/tests.rs)、[resume_new.rs](../../../crates/tui/src/app_task/resume_new.rs) |
| 普通 ReloadConfig 与 `/cli` `/skill` 投递失败 | `reload_config_store_failure_is_visible_and_emits_no_model_switch`、`cli_reload_dispatch_failure_keeps_live_config_and_reports_disk_save`、`skill_reload_dispatch_failure_keeps_live_config_and_reports_disk_save` | [tests_failures.rs](../../../crates/tui/src/worker/tests_failures.rs)、[cli_outcome_tests.rs](../../../crates/tui/src/app_loop_tests/cli_outcome_tests.rs)、[skill_outcome_tests.rs](../../../crates/tui/src/app_loop_tests/skill_outcome_tests.rs) |
| Env/XDG 配置缓存指纹 | `config_cache`；`configuration_cache_reuses_the_load_until_input_files_change`、`configuration_cache_tracks_active_env_transitions_and_files`、`configuration_cache_tracks_xdg_cli_and_skill_creation_edits_and_removals` | [config_cache.rs](../../../crates/worker/src/state/config_cache.rs)、[state/tests.rs](../../../crates/worker/src/state/tests.rs) |
| Ontology TUI 登记、原生目录同步与只读检查 | `platform_tui_entry_synchronizes_interactive_env_and_mode_model_configuration`、`native_changes_automatically_update_catalog_and_keep_read_permissions` | [ontology_catalog.rs](../../../crates/control/tests/ontology_catalog.rs) |

### 最终父流程验证记录

以下为最终父流程实际完成并通过的定向结果；各目标独立记录，不合并可能重复的
用例数量：

- Core：`config_modes`、`config_local_env`、`config_local_env_loading`、
  `config_contract` 均为定向通过。
- Store：`session_model_overrides` 定向通过。
- Session：`mode_model_requests`、`mode_model_persistence` 均为定向通过。
- Local：`model_override` 定向通过。
- Worker：`config_cache`、`configuration_cache` 均为定向通过。
- TUI：`model_menu`、`model_session_switch`、`env_model`、`session_only`、
  `model_override`、`worker`、`cli`/`skill`/`mcp` outcome、`app_task`、`ts_mirror`
  均为定向通过；独立集成目标 `model_switch_persist`（**2 passed**）和
  `session_switch_restore`（**3 passed**）全部通过。
- Ontology：`platform_tui_entry_synchronizes_interactive_env_and_mode_model_configuration`
  与 `native_changes_automatically_update_catalog_and_keep_read_permissions`
  两个定向测试均通过。
- `cargo check` 覆盖 Core、Store、Session、Local、TUI、Worker、Control，全部通过。
- 六个功能 crate（Core、Store、Session、Local、TUI、Worker）的 `--all-targets`
  Clippy（`-D warnings`）及 Control `ontology_catalog` Clippy 均通过。
- `cargo fmt --all --check`、`git diff --check` 与行数检查均通过。

### 本轮文档与目录核验

- 本次收尾仅更新本 changelog 的最终验证记录，未修改代码，也未重复运行上述已通过的
  测试或静态检查。
- 本文件 Markdown 本地相对链接定向检查无缺失，有效 UTF-8，行数低于 400 行；文件
  空白检查通过。
- 测试范围为上述定向目标及其直接检查；未运行全量 workspace、SPA 或全站 UI 测试，
  不声称全量 workspace 通过。
