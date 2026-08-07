Commit: (working-tree, pre-initial-commit)

# `ts -l` 全局化：跨 workdir 汇总 tmux 会话 + 已注册停止会话

## 背景
`opencode ts -l` 原先只展示**当前 workdir** 的 store 会话，并按 tmux 状态过滤：
tmux 会话来自全局 `tmux list-sessions`，但 store 侧只扫当前 workdir，导致
「在 repo A 启动、在 repo B 想看到全部会话」只能看到一半；且停止会话没有工作
目录信息，无法区分来自哪个项目。

本次将 `ts -l` 改为**全局面板**：tmux 侧天然全局（每个 live 会话带真实
`pane_current_path`），store 侧改为扫描 data root 下**所有** workdir 的
`opencoder.db`；停止会话仅展示被 ts 流程显式注册（种子行无 agent/model）且
**真正启动过**（有 preview/title）的会话——裸 `tui`/`run` 会话与从未启动的
空种子一律不出现。`ts -c`/`ts -r` 冷启动行为不变，无 schema 变更。

## 变更

### 1. core — 数据根目录提取
- **`crates/core/src/data_dir.rs`**：新增 `pub fn data_root() -> PathBuf`（=
  `<data_local>/opencoder`），`data_dir_for` 改为 `data_root().join(hash)`；
  同一算法不再在调用点漂移。新增 `data_root_is_opencoder_dir_under_data_local`
  测试。
- **`crates/core/src/lib.rs`**：与 `data_dir_for` 一同重导出 `data_root`。

### 2. cli — `ts -l` 全局面板
- **`crates/cli/src/ts/actions.rs`**：`ts_list` 重写为全局（忽略当前 workdir）：
  - `scan_all_stores(root)`：遍历 data root 下每个子目录，跳过非目录 / 缺
    `opencoder.db` / open 或查询失败者（`tracing::warn`，展示命令不因单个
    坏 store 目录而崩溃）。
  - `build_rows(store_items, tmux)`：live tmux 行**总是**展示（id 取自
    `opencode-<ulid>` 命名契约），路径 = `abbreviate_path(pane_current_path)`，
    优先用 store 的创建时间与 `/task` 预览富化；store 中无 live 孪生的会话仅当
    `is_registered` 时以 `(stopped)` 路径列出。
  - `is_registered(s)`：`agent.is_none() && model.is_none() && (preview 或
    title 非空)`——ts 种子行无 agent/model，普通 `tui`/`run` 会话首条消息即
    持久化二者，因此天然被排除；空种子（无 preview/title）同样排除。
  - `task_text`：preview（回退 title）截断 20 字符，空则 `(no task yet)`。
  - `sort_rows`：非停止在前 → path 升序 → created_at 降序（同 path 组内最新
    在前）。`GlobalRow { id, path, created_at, state, task }`。
  - 新增测试：`build_rows_unions_global_tmux_and_registered_stopped`（跨
    workdir 并集 + live 富化 + 停止行）、`build_rows_skips_never_started_seed_
    and_unregistered`（空种子与普通会话均不出现）、`scan_all_stores_skips_
    dirs_without_db_and_non_dirs`（tokio + 真实 `LibsqlStore` + tempdir，
    坏目录被跳过）、`sort_by_path_then_created_desc` 适配 `GlobalRow`；
    辅助 `mk_managed_at` / `mk_item`。
- **`crates/cli/src/lib.rs`** / **`crates/cli/src/ts/mod.rs`**：文档注释更新为
  全局语义——tmux-first、每行带 workdir 路径列、停止会话来自全部 store、
  显式注册（无 tmux 配置 / 未真正启动 = 不出现）。

## 行为对比
| 场景 | 旧 `ts -l` | 新 `ts -l` |
| --- | --- | --- |
| 其它 workdir 的 live tmux 会话 | 已展示（tmux 全局）但无路径列 | 展示 + `pane_current_path` 路径列 + store 富化 |
| 其它 workdir 的停止会话 | 不展示 | 展示为 `(stopped)`（仅 ts 注册且启动过） |
| 当前 workdir 的普通 `tui`/`run` 会话 | 展示 | 不展示 |
| 从未启动的 ts 空种子 | 展示（空行） | 不展示 |

## 测试覆盖（当次实跑）
- `cargo test --workspace` → **1927 passed / 0 failed**（本次新增 5 个用例全绿）
- 新增用例：opencoder-cli `build_rows_unions_global_tmux_and_registered_stopped` /
  `build_rows_skips_never_started_seed_and_unregistered` /
  `scan_all_stores_skips_dirs_without_db_and_non_dirs`（tokio+真实 LibsqlStore+
  tempdir）/ `sort_by_path_then_created_desc`（适配 GlobalRow）；
  opencoder-core `data_root_is_opencoder_dir_under_data_local`。
- `cargo clippy --workspace --all-targets -- -D warnings` → Finished，0 warning
- `cargo build --workspace` → Finished，0 error
- 手工验证（无 TTY 环境，tmux 可 list）：`ts -l` 输出 10 条跨 workdir live
  tmux 行（真实 `pane_current_path` 路径列 + `*`/`·` 标记）+ 2 条不同 store
  的 `(stopped)` 行（含 task 预览），普通 `tui` 会话（agent/model 非空）与
  从未启动的空种子均不出现；`ts` 启动路径本环境无 TTY 无法端到端（`os error 6`
  为既有环境限制，与本次变更无关）。
