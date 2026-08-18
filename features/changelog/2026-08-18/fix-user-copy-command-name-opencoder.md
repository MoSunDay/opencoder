Commit: (working-tree, post-860831d)

# 用户可见文案统一命令名为 `opencoder`

## Context

二进制名（`Cargo.toml`）、clap `#[command(name = "opencoder")]` 与既有 `resume with: opencoder -s {id}` 提示均为 `opencoder`，但仍有 7 处用户可见文案写着旧名 `opencode`（supervisor 退出提示、headless 空 prompt 用法、ts 子命令 5 处）。本轮统一为 `opencoder`；功能性标识（`TMUX_PREFIX = "opencode-"`、notepad 忽略项 `".opencode"`）**不动**——前者改名会导致已有 tmux 会话失联，后者是 gitignore 语义。

## Change Summary

- `crates/tui/src/supervisor.rs`：退出消息提取为纯函数 `exit_message(reason: Trip) -> String`（沿用 `trip_reason` 的纯函数模式），文案 `opencode:` / `` `opencode --continue` `` → `opencoder`；supervisor 线程改为调用该函数。
- `src/main.rs`：`require("")` 的 Usage 文案 `opencode "…"` / `opencode run "…"` → `opencoder`。
- `crates/cli/src/ts/actions.rs`（3 处）+ `ts/tmux.rs`（1 处）：`` `opencode tui` ``、`` `opencode ts -r <id>` ``、`` `opencode ts -l` `` 与 `LIST_LEGEND`（resume/delete/clean 三条）→ `opencoder`。
- `crates/cli/tests/cli_parse.rs`：argv[0] 占位符 `"opencode"` → `"opencoder"`（11 处，clap 忽略 argv[0]，纯测试一致性）。
- 明确不改：`ts/naming.rs` `TMUX_PREFIX`、`notepad/tree.rs` `".opencode"`、`config/env*.rs`（路径已是 `~/.opencoder/`）。

## Validation（当次实跑）

- `cargo test -p opencoder-tui --lib supervisor`：8 passed / 0 failed（含新 `exit_message` 单测）。
- `cargo test -p opencoder-cli`：全绿（ts / cli_parse 等全部套件，无旧文案断言残留）。
- `cargo test -p opencoder --bin opencoder`：2 passed / 0 failed（新 `require` 用法断言）。
- `cargo test --workspace`：**2974 passed / 0 failed**（review 时点）；终验复跑 **2985 passed / 0 failed**（并行迭代新增 ~11 条测试后全量复跑，仍全绿）。
- grep 门禁：`grep -rn '"opencode' crates/ src/ --include='*.rs' | grep -v opencoder` 仅剩 `TMUX_PREFIX = "opencode-"` 与 `".opencode"` 两处功能性标识。

## 测试覆盖表

| 测试 | 层级 | 断言 |
|---|---|---|
| `tui supervisor::tests::exit_message_leads_with_opencoder_and_resume_hint` | unit | 两种 Trip 下消息以 `opencoder:` 开头、含 `opencoder --continue`、含 reason；词边界断言不含裸 `opencode `/`opencode:` |
| `main tests::require_empty_prompt_error_advertises_opencoder_usage` | unit | `require("")` 错误含 `Usage: opencoder "your prompt"` 与 `opencoder run`；词边界断言旧名已消失 |
| `main tests::require_nonempty_prompt_passes` | unit | 非空 prompt 直接放行（回归保护） |
