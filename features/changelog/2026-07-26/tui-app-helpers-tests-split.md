Commit: (working-tree)

# refactor(tui): 拆分 app_helpers_tests 目录模块，单文件回到 800 行内

## 背景

`crates/tui/src/app_helpers_tests.rs` 在 image-pipeline 落地（新增 `open_store`
回归测试）后达 1088 行，超过 800 行迭代上限。该文件混装了四组互不相关的测试
（paste / mouse / input / model-store），需要按职责拆分。

## 变更

纯机械搬运，**无任何测试体改动、无重命名、无增删测试**（test 数恒为 24，全量 1206
通过）。把单文件转为目录模块：

- **`crates/tui/src/app_helpers.rs`**：模块声明 `#[path]` 由文件改为目录
  `app_helpers_tests/mod.rs`（1 行改动，运行期零影响）。
- **`crates/tui/src/app_helpers_tests/mod.rs`**（295 行）：`//!` 模块文档 +
  paste 测试（8）+ input 测试（6）+ model/store 测试（2）+ `mod mouse_tests;`。
  顶部 `use super::*;` 仍解析到 `crate::app_helpers`；裁剪掉搬运后不再使用的
  import（async_trait / Message / SessionEvent / 大部分 store 类型 / Rect），
  仅保留 `SessionMeta`。
- **`crates/tui/src/app_helpers_tests/mouse_tests.rs`**（797 行）：`StubStore`
  fixture + `parent_with_long_subagent`/`empty_hits`/`scroll_down` 三个辅助 +
  全部 8 个鼠标交互 `#[tokio::test]`，逐字搬运。子模块 `super` 解析到 `tests`
  而非 `app_helpers`，故改用 `use crate::app_helpers::*;` + 显式补入
  `ChatView`/`MouseHits` 等 app_helpers 私有 import 未导出的类型。
- 删除旧文件 `crates/tui/src/app_helpers_tests.rs`（原 1088 行）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 鼠标滚轮/点击（8 个，搬运未改） | `scrolldown_*` / `dbl_click_*` / `submit_btn_*` / `single_click_*` / `jump_btn_*` / `scrollup_*` / `thinking_header_*` | `app_helpers_tests/mouse_tests.rs` |
| paste 解析（8 个，搬运未改） | `paste_*` | `app_helpers_tests/mod.rs` |
| input/drain（6 个，搬运未改） | `clear_pending_inputs_*` / `ctrl_u_*` / `mk_input_with_images_*` / `drain_pending_images_*` | `app_helpers_tests/mod.rs` |
| model/store（2 个，搬运未改） | `reapply_session_model_*` / `open_store_creates_db_file_*` | `app_helpers_tests/mod.rs` |

- 全量回归：`cargo test --workspace` → **1206 passed / 0 failed / 0 ignored**（与拆分前一致）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished clean
- 行数：mod.rs 295、mouse_tests.rs 797、app_helpers.rs 793（均 ≤ 800）

## Impact Surface
- 仅触及 TUI 测试模块的物理组织；被测代码（`app_helpers.rs` 的 `pub(crate)` 函数）
  仅有 1 行 `#[path]` 声明变更，运行期行为零影响。
- 不影响：CLI/Web/session/store 边界、runner/prompt 契约。

## Related Docs
- [既有 changelog：app_helpers 抽取](tui-app-helpers-extract.md)
