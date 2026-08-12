Commit: (working-tree, pre-initial-commit)

# skill body 注入时附注源文件路径

## 背景

`Skill` 结构体已有 `source: PathBuf` 字段（保存 skill 文件的完整路径），但激活 skill
时只取 `body` 文本注入 system prompt，路径信息被完全丢弃。导致例如 `repo-local-memory`
的 body 引用 `[EXAMPLES.md](./EXAMPLES.md)`，注入后 `./` 相对于 agent CWD，agent 无法定位
这些同目录文件。

本次变更：激活 skill 时，将源文件完整路径以 `> Source: <path>` 块引用注记前缀至 body，
使 agent 能定位 skill 文件及其引用的同目录资产。

## 变更

### 新增纯函数
- **`crates/core/src/skill.rs:472`**：新增 `pub fn body_with_source(skill: &Skill) -> String`
  — 返回 `"> Source: {source_path}\n\n{body}"`，无副作用纯函数。
- **`crates/core/src/lib.rs:33`**：re-export `body_with_source`。

### 应用至 3 个 body 提取点
- **`crates/session/src/skill_resolve.rs:61`**：`bodies.push(sk.body.clone())` →
  `bodies.push(body_with_source(sk))`（headless / queue / steer 路径）。
- **`crates/tui/src/app_helpers.rs:388`**：`resolved_bodies.push(sk.body.clone())` →
  `resolved_bodies.push(opencoder_core::body_with_source(sk))`（TUI `$`-picker 路径）。
- **`crates/cli/src/run.rs:216`**：同上（headless `run` 子命令路径）。

### 回归测试更新
- **`crates/session/src/skill_resolve.rs`**：`skill()` 测试 helper 设置有意义的 `source`
  路径；4 处断言更新为含 `> Source:` 前缀。
- **`crates/tui/src/app_helpers_tests/skill_apply.rs`**：6 处断言从精确相等改为
  `starts_with("> Source: ")` + `ends_with(<body>)`。
- **`crates/tui/src/skill_persist.rs`**：同上策略更新断言。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| body_with_source 输出包含 source 路径前缀 | `body_with_source_prefixes_path_before_body` | `crates/core/src/skill.rs` |
| discover 后 body_with_source 含磁盘路径注记 | `body_with_source_emits_path_annotation_then_body` | `crates/core/tests/skill_contract.rs` |
| session skill_resolve 路径注入正确 | `resolves_single_skill_and_strips_token` 等 4 项 | `crates/session/src/skill_resolve.rs` |
| TUI skill_apply 路径注入 | `apply_skill_tokens_resolves_and_activates_known_skill` 等 5 项 | `crates/tui/src/app_helpers_tests/skill_apply.rs` |
| TUI skill_persist 路径注入 | `resolve_persist_persists_and_activates_known_skill` | `crates/tui/src/skill_persist.rs` |

- 全量回归（隔离后，仅本变更）：`cargo test --workspace` → `2378 passed; 0 failed`
  （1 个 pre-existing 挂起测试 `handoff_tracks_supervisor_handle_for_cleanup` 已跳过，
  与本变更无关——`bg.rs` 无任何改动）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 行数：`crates/core/src/skill.rs` 798 ≤ 800。

## Impact Surface

- 用户可感知：激活 skill 后注入的 system prompt 包含 `> Source: <path>` 注记，agent 能
  定位 skill 引用的同目录文件（如 `EXAMPLES.md`、`TEMPLATES.md`）。
- Resume 路径不受影响：持久化的 body 含路径注记，`infer_skill_names`（检查前 200 字符中的
  skill 名）路径本身含 skill 名（如 `/root/.opencoder/skills/ssh-pty/SKILL.md` 含 `ssh-pty`），
  反而更可靠。
- Web API 路径：web 客户端直接传 body 字符串，不经 `body_with_source`，不获路径注记
  （已知限制，thin client 架构，可后续处理）。

## Related Docs

- [agents/core](../../../agents/core/index.md)（`Skill` 结构体、skill 发现与注入）
- [features/index.md](../../index.md)（skill 选择 / 内置 skill 清单）
