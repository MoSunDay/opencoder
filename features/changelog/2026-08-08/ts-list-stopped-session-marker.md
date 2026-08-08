# fix(cli): ts -l 显示 stopped 会话 + 状态 marker 可读化

## 背景

`opencode ts -l` 之前不显示 stopped 会话。根因：

- ts 会话的判定谓词 `is_ts_seed` 为 `agent.is_none() && model.is_none()`；
- 但 ts 会话通过 `/act`、`/plan`、`SwitchAgent` 切换模式时走
  `persist_agent`（`crates/session/src/control_cmd.rs:169`），该方法**只 patch
  `agent`**，`model` 保持 NULL（seed 用 INSERT OR IGNORE 落库，仅首次写入）；
- 因此模式切换后的 ts 会话不再满足 `is_ts_seed`，退出后从 stopped 列表消失。

会话创建时的持久化事实：普通 `tui`/`run` 首条消息必然持久化 `model`；ts 的
seed 在 TUI 启动前插入，`model` 留 NULL。所以 **ts 的持久标记是 `model IS
NULL`**，而非"agent 与 model 都为空"。

## 变更

`crates/cli/src/ts/actions.rs`：

- **`is_ts_seed` → `is_ts_owned`** 重命名，条件改为 `s.model.is_none()`——
  mode-switched 的 ts 会话（`agent` 已落库、`model` 仍 NULL）依然归属 ts。
  全部 6 处调用点同步更新（约 55/280/288/329/405/468/493 行）。
- **stopped marker 由 `" "`（不可见空格）改为 `"-"`**（约 178 行），列对齐
  不再依赖肉眼不可见的占位。
- **图例**改为 `* attached  · live(detached)  - stopped`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 回归：mode-switched ts 行（model NULL）被列出；普通会话（model 已落库）被排除 | `build_rows_skips_never_started_seed_and_unregistered` | `cli/src/ts/actions.rs` |
| 图例不宣传已移除 flag、仍列 live 命令 | `list_legend_has_no_removed_flags` | `cli/src/ts/actions.rs` |

手动验证：`opencode ts -l` 显示 stopped 会话 `01KZDRY7` 为 `-`。

## 回归

- `cargo test -p opencoder-cli` → **61+27+3+1 passed / 0 failed**
- 本次定位到 cli crate 全绿；全量 workspace 回归受并发 TUI 重构的编译中间态
  阻塞，待其合并后补跑。

## 影响面

- `agents/cli/index.md`：`ts -l` marker 描述同步为 `*` attached / `·`
  live(detached) / `-` stopped，并补充 ts-owned 谓词 = model NULL。
- `features/changelog/2026-08-04/ts-store-first-unified-management.md`：旧 marker
  描述同步（空=stopped → `-`=stopped），并注明 mode-switched ts 会话保持
  model NULL（持久标记 = model IS NULL）。
