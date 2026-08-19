Commit: (working-tree, post-860831d)

# 配置面修复批：OPENAI_BASE_URL 同步活动 provider、redact_json、api_key 报错指路、has_editable_key 补全、空 model 守卫、context-limit 助手、env capture 告警、skills_dir 不再写 CWD

## 背景

八个独立但同属 config/skill 面的缺陷（bug #2/#3a/#4/#5/#6a/#7/#11/#12）：

1. `OPENAI_BASE_URL` 在 `OPENCODER_MODEL` **之前**应用且只写顶层 `provider.base_url`——env 切换活动 provider 后，`providers[<active>].base_url` 保持文件旧值，env 覆盖在端点解析时被静默忽略。
2. 没有 `api_key` 脱敏器：config JSON（含 env capture 预览）原样回显会泄 key。
3. `api_key_for` 报错只有 `missing OPENAI_API_KEY`，不指明 provider，也不列三条配置途径。
4. `has_editable_key` 漏识别 `agent` / `stream_idle_timeout_secs` / `task_timeout_secs` / `replay_timeout_secs` / `subagent_drain_secs` / `output_streamline` / `tool_guard`：一个只带这些键的 config.json 不算"可编辑"，`save_target` 会新建第二个 opencoder.json 而不是合并进用户已有文件。
5. `is_suspicious_model("")` 为 false：空 model 可被 `Config::save` 落盘（每个请求都以 `model:""` 失败）。
6. `OPENCODER_CONTEXT_LIMIT=abc` 被静默忽略。
7. `capture_into` 对损坏候选（不可读/不可解析/非对象）静默跳过，用户不知道 env 为何"丢"键。
8. `skills_dir()` 无 HOME 时回退到**相对路径** `./.opencoder/skills`，seeding 把技能文件写进任意 CWD。

## 变更

- **`crates/core/src/config/env.rs`** — `apply_env` 重排：`OPENCODER_MODEL`/`OPENCODER_SMALL_MODEL` 先于 base_url；`OPENAI_BASE_URL`（非空）同时写顶层 `provider.base_url` 与活动 provider 注册表项（按 `Config::provider_id` 同款派生：`pfx/model` → `pfx`，无 `/` → `openai`；同样的 `trim_end_matches('/')` 归一化；注册表无该 entry 时只写顶层、不 panic）。
- **`crates/core/src/config/redact.rs`（新，pub mod）** — 纯函数 `pub fn redact_json(value: &serde_json::Value) -> serde_json::Value`：递归深拷贝，键名恰为 `"api_key"` 且值为字符串 → 前 4 字符 + `"***"`（≤4 字符 → `"***"`），非字符串值与其余键原样保留。cli/web 可经 `opencoder_core::config::redact::redact_json` 使用。
- **`crates/core/src/config.rs`** — `api_key_for` 报错改为：`missing API key for provider `<name>`: set `providers.<name>.api_key`, top-level `provider.api_key`, or the `OPENAI_API_KEY` env var`。`is_suspicious_model` 改 `pub` 且空串 → true（load 侧 `warn_if_suspicious_model` 不变：fresh install 默认 `openai/gpt-4o-mini` 非空，只有显式坏配置才告警；save 侧守卫随之拒绝空 model）。本文件净增 2 行（`pub mod redact;` + 报错扩行），798→800。
- **`crates/core/src/config/merge.rs`** — `has_editable_key` 补：4 个标量超时键 `contains_key`；`agent` 非空对象（覆盖 `agent.default` 字符串与未来子键）；`output_streamline`/`tool_guard` 沿 `keymap` 先例用非空对象判定（二者所有子键均可编辑）。
- **`crates/core/src/config/envs.rs`** — `capture_into` 候选循环改为 match 三分支：NotFound 静默；读失败（非 NotFound）/解析失败/非对象 → `tracing::warn!`（path + 原因）后跳过，永不硬失败。新增纯助手 `json_kind_name`。
- **`crates/core/src/skill.rs` + `skill/seed.rs` + `tool_deps.rs`、`crates/session/src/skill_context.rs`、`crates/cli/src/install_tools.rs`** — `skills_dir() -> Option<PathBuf>`（无 HOME → None，绝不回退相对路径）。`discover`/`catalog_entries` None → 空集合；`reminder_text` 展示回退为字面 `~/.opencoder/skills`；seeding（内建与 dep-gated）None → warn 一次后跳过（新增 `seed_packs_at_home` 承载跳过路径）；`check_tool_deps` 哨兵探测 None → false；`install_tools_run` None → 打印原因并以退出码 1 跳过。

## 测试清单

- core 集成（`crates/core/tests/config_providers.rs`）：`openai_base_url_env_overrides_active_provider_registry_entry`（注册表项 == env 值、顶层 == env 值、非活动 entry 不动、resolve_endpoint 用 env 值）、`openai_base_url_env_without_registry_entry_updates_legacy_only`（无 entry 只写顶层、不 panic）、`openai_base_url_env_registry_sync_normalizes_trailing_slash`、`api_key_error_names_provider_and_all_config_avenues`（报错含 provider 名、`OPENAI_API_KEY`、两条配置键路径）。
- core 集成（`crates/core/tests/config_contract.rs`）：`save_refuses_empty_model_value`（Err + 零落盘）。
- core 单测（`crates/core/src/config/tests.rs`）：`empty_model_is_suspicious`（重命名自 `empty_model_is_not_suspicious`，断言翻转）、`has_editable_key_recognizes_timeout_and_agent_keys`（8 个新键全绿）、`has_editable_key_ignores_empty_agent_and_object_keys`（空对象/非对象/无关键为负例）。
- core 单测（`crates/core/src/config/redact.rs` 内嵌 6 test）：长 key 前 4+***、≤4 字符 → `***`、数组嵌套对象、兄弟键原样且入参不被改、非字符串 api_key 原样、非对象根原样。
- core 单测（`crates/core/src/config/env.rs` 内嵌）：`parse_context_limit_accepts_plain_u64` / `parse_context_limit_rejects_garbage`（"abc"/""/"-1"/"1e5"/" 8192"/"8192 " → None；无 trim 语义 pinning。warn 断言因无日志捕获基建而省略，仅 pin 助手 + no-panic）。
- core 单测（`crates/core/src/config/envs.rs`）：`capture_skips_corrupted_candidates_and_keeps_valid_keys`（不可解析 + 非对象两个坏候选：capture Ok，输出仅含合法键）。
- core 单测（`crates/core/src/skill/seed.rs`）：`seeding_without_home_dir_skips_without_writing`（None → 不 panic、不写 `./.opencoder/skills`）。
- core 集成（`crates/core/tests/skill_contract.rs`）：`skills_dir_points_at_global_home`（隔离 HOME → Some，后缀 `.opencoder/skills` 且落在临时 home 内）、`skills_dir_without_home_is_none_or_absolute_never_cwd`（去 HOME 后 Some(绝对路径) 或 None——`dirs` 有 getpwuid 回退无法稳定强制 None，pin 的不变量是"绝不相对路径/CWD"）。
- 回归：`cargo test -p opencoder-core` 全绿（168 lib + 集成全过）；session/cli/web 见提交说明（同仓并行任务在途）。

## Impact Surface

- 公共 API 新增：`opencoder_core::config::redact::redact_json(&serde_json::Value) -> serde_json::Value`；`opencoder_core::config::is_suspicious_model(&str) -> bool` 由 `pub(crate)` 转 `pub`（签名不变）。
- 公共 API 变更：`opencoder_core::skills_dir()` 返回 `PathBuf` → `Option<PathBuf>`（调用方全部内联适配，无外部 consumer）。
- 行为变更：`OPENAI_BASE_URL` 现在也覆盖活动 provider 注册表项；`Config::save` 拒绝空 `model`；`has_editable_key` 对 7 个新键返回 true（影响 `save_target` 路由——旧行为是错误地新建文件）；env capture 对坏候选打 warn。
