Commit: (working-tree, pre-initial-commit)

# fix(tui): 消除 theme::set_then_current_theme 并发竞态导致的间歇性回归失败

## 背景
`cargo test --workspace` 间歇性失败于 `theme::tests::set_then_current_theme`：断言
`left == right` 得到 `left: Dark, right: Light`。根因是 `THEME` 是进程级全局
`OnceLock<RwLock<ThemeKind>>`，而 crate 内大量并行测试（`render_tests/*`、
`chat_tests/*`、`theme` 模块自身的 `context_meter_*` 等）都调用 `set_theme`。`set_then_current_theme`
是唯一设置 `Light` 的测试，在它 `set_theme(Light)` 与 `current_theme()` 之间，任一并发测试的
`set_theme(Dark)` 落地即破坏断言。单独跑全过，全量并行才复现。

## 变更
### tui theme 测试竞态修复
- **`crates/tui/src/theme.rs`**（`set_then_current_theme`，~287 行）：在临界区内持有
  `THEME` 的**独占写锁**并直接读写存储值，而非调用非重入的 `set_theme`/`current_theme`
  （二者各自再取同一把 `RwLock`，持锁期间调用会自死锁）。独占写锁使所有并发的
  `set_theme`/`current_theme` 阻塞至本测试恢复默认值，从而确定化、无死锁。`palette()`
  的真实配色逻辑仍由既有 `palette_*` 测试覆盖。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| theme 存储往返（确定化） | `set_then_current_theme` | crates/tui/src/theme.rs |

- 全量回归：`cargo test --workspace` → 1898 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净
- 行数：crates/tui/src/theme.rs 迭代中，未超限

## Impact Surface
- 仅影响测试；运行时主题切换行为不变。
- 不影响：CLI/Web/session/store 边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
