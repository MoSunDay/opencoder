Commit: (working-tree, pre-initial-commit)

# test(tui): perf_long_session 时序断言限定 Release，修复 debug 构建 flaky 失败

## 背景
`crates/tui/tests/perf_long_session.rs` 的三个 `viewport_build_and_slice_{1k,5k,10k}_blocks`
用例断言 `ViewportCache::build` 的**绝对耗时**（<500ms / <1000ms / <3000ms）。这些阈值是
为 **Release（优化）构建**校准的，但用例在默认 `cargo test`（debug，unoptimized）下执行；
`build` 是 O(n) 的 flatten + 字符串格式化，在 debug 下慢 5–10×，导致阈值被系统性击穿：

- debug：`5k_blocks` 稳定失败（build ~1.3–1.7s vs 1s 阈值），`1k/10k` 在并行负载下偶发失败；
- release：全部稳定通过（4/4，~0.3s）。

已取证：在干净工作树（无任何业务改动）上复现相同失败 → 与功能改动无关，自 `c55bb02`
引入以来即为既有 flaky。前序 changelog `tui-resize-log-tty-guard-robustness.md` 已将其
标记为“建议在独立任务中硬化（放宽阈值或限定 Release）”，本提交即落实该建议。

## 变更
### perf 时序断言限定 Release（`crates/tui/tests/perf_long_session.rs`）
- **`crates/tui/tests/perf_long_session.rs`**：将三个 `build_ms` 绝对耗时断言包进
  `if !cfg!(debug_assertions) { … }`。debug 下 `build`/`visible_window` 代码路径**仍执行**
  （可捕获 panic/逻辑错误），仅跳过对 debug 无意义的绝对阈值；`slice_us` 与
  `per_frame_cost_bounded_by_visible_h_not_block_count`（相对/比例不变量）在所有构建下
  保持生效。遵循 rules/03-test-pyramid.md：绝对时序 perf 基准属独立 tier，仅在 Release 下断言。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| debug 下代码路径仍执行、不 panic | `viewport_build_and_slice_{1k,5k,10k}_blocks` | `crates/tui/tests/perf_long_session.rs` |
| 比例不变量（O(visible_h)）在所有构建生效 | `per_frame_cost_bounded_by_visible_h_not_block_count` | `crates/tui/tests/perf_long_session.rs` |

- debug：`cargo test -p opencoder-tui --test perf_long_session` → 4 passed / 0 failed。
- release：`cargo test --release -p opencoder-tui --test perf_long_session` → 4 passed / 0 failed（~0.3s）。
- 全量回归：`cargo test --workspace` → 1204 passed / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 行数：`perf_long_session.rs` 184 ≤ 800 ✅。

## Impact Surface
- 用户/CI：`cargo test --workspace`（debug）不再被 perf 时序用例偶发拖红；Release 仍保留
  绝对耗时回归护栏。
- 不影响：`Store`/`ChatStream`/session/LLM 边界；无产品行为变更（仅测试断言门控）。

## Related Docs
- 落实 [tui-resize-log-tty-guard-robustness.md](./tui-resize-log-tty-guard-robustness.md) 中的 perf 硬化建议
- [agents/tui](../../agents/tui/index.md)
