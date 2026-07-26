Commit: (working-tree)

# refactor(tui): 抽取 resize/store 辅助函数到 app_helpers，压低 app.rs 行数

## 背景

`crates/tui/src/app.rs` 在 idle-resize 轮询安全网（`tui-resize-log-tty-guard-robustness.md`）
落地后逼近 800 行迭代上限（825 行）。其中 store 打开块、`size_changed`、Resize 事件处理、
idle-resize 轮询四段逻辑都是可独立测试/可复用的纯过程，但埋在 `run_app` 的 select 臂里，
既无法单独测试，也推高了主文件行数。

## 变更

纯行为保持型抽取，无运行期语义改动：

- `crates/tui/src/app_helpers.rs`
  - `size_changed(prev, cur) -> bool`（从 `app.rs` 原样移入，纯函数）
  - `open_store(workdir) -> Arc<dyn Store>`（把 `app::run` 内的「建 data_dir + open LibsqlStore」
    块抽成独立 async fn；`.ok()` 忽略 mkdir 失败的兜底语义保留）
  - `on_resize_event(terminal)`（封装 Resize 臂的 `terminal.autoresize()`）
  - `poll_idle_resize(terminal, last_size) -> bool`（封装每帧 ioctl 轮询 + autoresize + dirty 标记）
  - `pub(crate) use` 重导出列表同步更新
- `crates/tui/src/app.rs`
  - 删除上述四段内联实现，改为调用抽取出的函数；`LibsqlStore` import 下沉到 `app_helpers`
  - 行数 825 → 796（< 800 上限）
- `crates/tui/src/app_tests.rs`
  - 3 处 `size_changed` 测试的 `use` 路径由 `crate::app::` 改为 `crate::app_helpers::`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `open_store` 创建 DB 文件（新增） | `open_store_creates_db_file_in_workdir_hashed_data_dir` | `crates/tui/src/app_helpers_tests.rs` |
| `size_changed` 维度变化（路径迁移后保留） | `size_changed_detects_dimension_change` | `crates/tui/src/app_tests.rs` |
| `size_changed` 不变返回 false（路径迁移后保留） | `size_changed_false_when_unchanged` | `crates/tui/src/app_tests.rs` |
| `size_changed` 无先验返回 true（路径迁移后保留） | `size_changed_true_when_no_prior_reading` | `crates/tui/src/app_tests.rs` |

- 全量回归：`cargo test --workspace` → 全绿 / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：app.rs 796、app_helpers.rs 793（均 < 800）

### 单测豁免说明（rules/03）

`on_resize_event` / `poll_idle_resize` 操作 ratatui `Term`（`terminal.autoresize()` /
`terminal.size()` 的 ioctl），需要真实 tty/pty，无纯函数分支可供离线断言——它们是对
ratatui/crossterm 调用的薄封装，可观测行为仅存在于活动终端。`open_store` 是唯一带可观测
副作用（落盘 DB 文件）的新 fn，已补 tempdir 单测覆盖。这两者不另开 unit 测试。
