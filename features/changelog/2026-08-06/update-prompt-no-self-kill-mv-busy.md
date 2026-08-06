# fix(cli): update 提示词禁止 kill 自身、busy 用 mv 原子替换

## 背景

`opencoder update` 经内置提示词委托代理执行自更新（clone → build → 替换 PATH
二进制）。原提示词仅笼统说「注意处理 busy 情况」，但更新任务**必然运行在正在执行的
opencoder 进程内部**——代理若用 `kill` / `pkill` / `killall` 终止 opencoder，会把
自己一起杀掉，导致更新流程中断、用户会话直接消失。busy（ETXTBSY / Text file busy）
也应以 `mv` 原子替换规避，而非 `rm` 等破坏性手段。

## 变更

### `crates/cli/src/update.rs`

- `UPDATE_PROMPT` 常量补充三条硬约束（`update.rs:13-20`）：
  - 明确本次更新必然运行在正在执行的 opencoder 进程内部。
  - 绝对不要杀死或终止当前 opencoder 进程本身（禁止 `kill` / `pkill` / `killall`）。
  - 替换二进制遇 busy（ETXTBSY）必须用 `mv` 方式处理：先 mv 到临时名再 mv 覆盖目标，
    或先 mv 旧二进制移走再写入新的；禁止 `rm` / 杀进程等破坏性手段。
- 同步更新常量上方 doc 注释（`update.rs:6-12`），补述「不杀自身 + busy 用 mv」保证。

## 测试覆盖

纯提示词变更，无新逻辑分支；既有解析测试覆盖子命令注册：

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `update` 子命令仍正常解析 | `update_subcommand` | [crates/cli/tests/cli_parse.rs](../../../crates/cli/tests/cli_parse.rs) |

- 全量回归：`cargo test --workspace` → 全绿
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：`crates/cli/src/update.rs` 28 ≤ 800

## Impact Surface

- 仅影响 `opencoder update` 委托给代理的指令文本，运行时行为不变。
- 不影响：CLI/Web/session/store/其它子命令。

## Related Docs

- [agents/cli](../../agents/cli/index.md)
- [既有 changelog：update 子命令初版](../2026-07-31/update-subcommand.md)
