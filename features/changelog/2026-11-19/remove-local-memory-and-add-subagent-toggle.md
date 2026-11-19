# 移除本地仓库记忆，`/config` 新增 `subagent` 开关

一句话：删除任务后记忆维护链路（session runner `local_memory` 模块、
`local_memory` 配置键与 `/config` 开关、内置技能 `repo-local-memory` /
`repo-local-dreaming` 及 seed）；`/config` 表单改为提供 `subagent` 开关
（Config 键 `subagent: Option<bool>`，默认开启，关闭后不启动子代理）。

## 背景与结论

两项变更都在本地会话基建内闭环，不改存储 schema、Server 协议与依赖集合。

### 1. 移除 local memory

- 记忆维护会话流程 `crates/session/src/runner/local_memory/` 整目录删除，
  任务完成后不再派生记忆维护会话。
- Config 删除 `local_memory` 键（原默认关闭）；TUI `/config` 表单删除
  对应开关。
- 内置技能 `repo-local-memory`（SKILL/EXAMPLES/TEMPLATES）与
  `repo-local-dreaming`（SKILL）连同 `crates/core/src/skill/seed.rs` 的
  seed 条目一并移除，技能契约测试同步。

### 2. `/config` 新增 `subagent` 开关

- Config 新增 `subagent: Option<bool>`（缺省 = 开启），访问器
  `subagent_enabled()`；TUI `/config` 表单提供开关，保存时写回
  `opencoder.json`。
- 关闭后不启动子代理，三处口径一致：
  - 工具 schema：`task` 不进 advertised 工具集，模型侧不再看到
    （`crates/session/src/runner/llm_call.rs`）；
  - 派发：幻觉 `task` 调用在创建任何子会话之前直接拒绝
    （`crates/session/src/runner/execute.rs`）；
  - 恢复：resume 重放跳过子代理启动（`crates/session/src/resume.rs`）。

## 变更清单

- `crates/session/src/runner/local_memory/` — 删除。
- `crates/core/assets/skills/repo-local-memory/`、
  `crates/core/assets/skills/repo-local-dreaming/` — 删除。
- `crates/core/src/skill/seed.rs` — seed 清单移除两个内置技能。
- `crates/core/src/config.rs`、`crates/core/src/config/merge.rs` — 删
  `local_memory` 键，增 `subagent` 键（显式默认开启）。
- `crates/session/src/runner/llm_call.rs`、`crates/session/src/runner/execute.rs`、
  `crates/session/src/resume.rs` — `subagent` 开关三处强制点。
- `crates/session/tests/subagent_toggle.rs` — 新增集成回归。
- `crates/tui/src/model_menu/` — `/config` 表单开关由 `local-memory`
  换为 `subagent`（`config_form.rs` / `patch.rs` / `view.rs`）。
- `features/index.md`（移除「本地仓库记忆」卡与 `base/` 目录注记）、
  `features/base/local-memory/index.md`（删除）、
  `agents/runtime/session/index.md`、`agents/frontend/tui/index.md` —
  索引同步。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `subagent` 配置默认开启与显式合并 | `subagent_is_an_explicit_default_on_config_toggle` | `crates/core/src/config/merge.rs` |
| 关闭后拒绝 `task` 派发且不建子会话 | `disabled_toggle_denies_task_call_and_starts_no_child` | `crates/session/tests/subagent_toggle.rs` |
| 关闭后隐藏 `task` 工具 | `disabled_toggle_hides_task_from_advertised_tools` | 同上 |
| 默认开启回归 | `default_config_advertises_task_and_dispatches` | 同上 |
| 关闭后恢复不重放子代理 | `replay_cancelled_tasks_skips_children_when_subagent_disabled` | 同上 |
| `/config` `subagent` 开关保存 | `config_form_subagent_toggle_is_saved` | `crates/tui/src/model_menu/tests/config_tests.rs` |
| 内置技能集契约（seed 清单与工作流） | `skill_contract`（seeding/workflows） | `crates/core/tests/skill_contract/` |

## 定向回归（实测）

- `opencoder-session`：clippy 零警告；
  `cargo test -p opencoder-session --test subagent_toggle` 4 passed；
  `--test subagent --test plan_subagent_guard` 13 passed。
- `opencoder-core`：clippy 零警告；`--lib config` 143 passed。
- `opencoder-tui`：clippy 零警告；`--lib model_menu` 102 passed；
  `--lib command::` 49 passed。
- `opencoder-worker`：`--test platform` 9 passed。
- `opencoder-web`：`--test web_api_ops --test web_title` 18 passed。
- `opencoder-project`：`--test plan_and_execute` 15 passed。
- `opencoder-dag-runtime`：`--test run_loop` 14 passed（dag-runtime clippy
  存量 `needless_range_loop` 告警与本改动无关）。

**测试范围未跑全量**：未运行 `cargo test --workspace`、SPA 全量测试或
全站 UI 验收；仅运行上述定向用例。文档侧仅核对本 changelog 与索引链接。

## 回滚

- 直接 revert 本次提交；无 schema、Server、配置存储迁移（`local_memory`
  键残留于旧 `opencoder.json` 时按未知键忽略，`subagent` 缺省回开启）。
- 回滚时删除本 changelog。

