Commit: (working-tree, pre-initial-commit)

# 主题色全局迁移——所有语义色统一走 theme::*() 函数

## 背景
上一轮（ed663e5）落地了 `/config` dark/light 切换基础设施，但 light 主题只对 theme.rs 辅助函数（边框/高亮/仪表/chip）与 `/config` 表单生效——chat/render/markdown/menu 等核心模块仍直接硬编码 `Color::*`，切换 light 后这些语义色不跟随，导致半明半暗。本次将全部语义色迁移到 `theme::*()` 函数，使 light 主题覆盖全局。

## 变更
### 全局语义色迁移（131 处硬编码 → theme 函数）
按颜色语义映射（Cyan→accent / Yellow→warn_color / Green→ok_color / Red→err_color / Blue→info_color / DarkGray→muted / Gray→subtle / White→text / Magenta→local_color），将分散在各模块的 `Color::*` 字面量替换为 `crate::theme::*()`：

| 颜色 | theme 函数 | 替换处 |
|------|-----------|--------|
| DarkGray | `muted()` | 41 |
| Yellow | `warn_color()` | 29 |
| Red | `err_color()` | 19 |
| Cyan | `accent()` | 19 |
| Green | `ok_color()` | 11 |
| Gray | `subtle()` | 5 |
| Blue | `info_color()` | 4 |
| White | `text()` | 2 |
| Magenta | `local_color()` | 1 |

涉及 17 个源码模块（chat/render/app_loop/markdown/menu/task/command/session_ui/app/app_helpers/app_task/model_menu/view/cache_salt_menu/view/queue_panel/help/welcome/model_session_switch），各文件 +14 处 chat.rs 最密集。

### 刻意保留（两主题值相同 / 非语义色）
- **`crates/tui/src/model_menu/view.rs`**：Save/Cancel 按钮的 `Color::Green`/`Color::Red` 作为按钮底色（Green/Red bg），dark/light 两套 palette 取值相同，无需迁移。
- `Color::Black`（饱和背景上的前景字）、`Color::Rgb`、`Color::Reset`、`Color::Indexed`：非语义色，保持原样。

### 测试颜色断言加 set_theme guard
- **`crates/tui/src/render_tests/chips.rs`** / **`status_ctx.rs`** / **`crates/tui/src/chat_tests/tool_collapse.rs`** / **`plan_card.rs`**：断言颜色的 `#[test]` 开头加 `set_theme(ThemeKind::Dark)` 守卫（4 处），确保测试取 dark palette 基线值，与原 `Color::*` 字面量断言一致，不受全局主题状态影响。断言本身保留 `Color::X` 字面量（dark 基线语义色与 const 等价）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| accent chip 着色（dark 基线） | chip 系列断言 | crates/tui/src/render_tests/chips.rs |
| 状态栏 ctx 着色（dark 基线） | status_ctx 断言 | crates/tui/src/render_tests/status_ctx.rs |
| 工具折叠着色（dark 基线） | tool_collapse 断言 | crates/tui/src/chat_tests/tool_collapse.rs |
| plan 卡片着色（dark 基线） | plan_card 断言 | crates/tui/src/chat_tests/plan_card.rs |

- 全量回归：`cargo test --workspace` → 1406 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：chat.rs 788 / render.rs 761 / app_loop.rs 798 / session_ui.rs 792 / model_menu/view.rs 557（均 ≤ 800）

## Impact Surface
- 切换 light 主题后，chat/render/markdown/menu/task 等全部模块的语义色随之变为白底可读 palette，不再半明半暗。
- 不影响：dark 主题外观（dark 下 `theme::*()` 与原 `Color::*` const 等价，视觉零变化）、Store/LLM/session 边界、CLI/Web。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [上一轮主题切换 changelog](./config-theme-toggle-dark-light.md)
