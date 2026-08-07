Commit: (working-tree, pre-initial-commit)

# feat(session/bash): silent `timeout N;` prefix override (clamped 120–600s)

## 背景

bash 工具的前台死线由固定常量驱动（`BASH_TIMEOUT_SECS` 真实 130s / 测试 1s；
`BASH_TIMEOUT_DISPLAY_SECS` 文案 120s / 测试 1s），模型无法在单条命令上放宽/收窄
它。历史上曾把 `timeout` 暴露为 schema 属性，但那是个已知失败模式（模型随意放大 →
handoff 被旁路），已被移除并加 `parameters_schema_hides_timeout_from_model` 守卫。

本变更在不重新暴露 schema 的前提下，让模型经**命令字符串前缀** `timeout <secs>;`
静默表达期望死线：纯字符串匹配（非正则）解析前缀 → 剥离前缀执行 → 覆盖前台死线与
handoff/killed 文案数字，并 clamp 到 `[120, 600]` 秒。

## 设计要点

- **纯函数解析** `parse_command_timeout(command) -> Option<(u64, &str)>`：仅匹配命令
  *前缀* `timeout` + ≥1 空白 + ≥1 ASCII 数字(≥1) + 可选空白 + `;`；返回未 clamp 的秒数
  与分号后的剩余命令。不匹配返回 `None`（`echo "timeout 5"`、`timeout abc;`、
  `timeout 60` 无分号、`timeout 0;` 均为 None）。不支持 `5m`/`2h` 后缀（口径=秒）。
- **clamp** `clamp_timeout(raw)` → `raw.clamp(MIN, MAX)`。`BASH_TIMEOUT_MIN_SECS` 生产
  120s / 测试 1s（cfg 门控，对齐既有 `BASH_TIMEOUT_SECS` 的测试折叠模式，使 override
  路径可快速验证）；`BASH_TIMEOUT_MAX_SECS` 始终 600s。
- **执行接入**（`execute()`）：解析后 `timeout_secs == display_secs == clamp(raw)`；
  无前缀则回退默认常量。前缀从实际执行命令剥离（避免 GNU `timeout` 无子命令时退出
  125 + stderr 噪音）；剥离后剩余为空 → 原有 `empty command` 错误分支。
- **schema 不变**：仍只暴露 `command`/`workdir`，`timeout` 不可见。

## 变更文件

- `crates/session/src/tools/bash.rs`（573 → 799 行，< 800 上限）
  - 新增常量 `BASH_TIMEOUT_MIN_SECS` / `BASH_TIMEOUT_MAX_SECS`
  - 新增纯函数 `parse_command_timeout` / `clamp_timeout`
  - `execute()`：override 解析 + 剥离前缀 + 三处替换（`run_cmd`、`Duration`、文案
    `{display_secs}`）

## 测试覆盖

| 功能 | 测试名 | 文件 | 层 |
|------|--------|------|----|
| 前缀解析：正常 | `parse_command_timeout_basic` | `crates/session/src/tools/bash.rs` | unit |
| 前缀解析：多余空白/Tab | `parse_command_timeout_extra_whitespace` | 同上 | unit |
| 前缀解析：大数不 clamp | `parse_command_timeout_large_number_unclamped` | 同上 | unit |
| 前缀解析：非法输入全拒 | `parse_command_timeout_rejects_invalid` | 同上 | unit |
| clamp 落在 [MIN,MAX] | `clamp_timeout_stays_in_band` | 同上 | unit |
| override=2 触发 handoff + 文案 2s | `bash_timeout_override_triggers_handoff` | 同上 | integration(unix) |
| override=5 放宽死线、不 handoff | `bash_timeout_override_widens_deadline` | 同上 | integration(unix) |
| `timeout 7;` 空剩余 → empty 错误 | `bash_timeout_override_empty_rest_errors` | 同上 | integration |
| 回归：schema 仍隐藏 timeout | `parameters_schema_hides_timeout_from_model` | 同上 | unit |

- 全量回归：`cargo test --workspace` → **2017 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build -p opencoder-session`（非 test，触发编译期断言）→ 零错误
- 行数：bash.rs 799 行（< 800）
