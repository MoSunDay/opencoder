# rustfmt 1.11 全仓机械重排

一句话：用 rustfmt 1.11.0-stable（rustc 1.100.0）对全仓 Rust 文件做一次纯格式
重排；首批 119 个已跟踪文件，随后补齐 9 个语义/遗漏文件，合计 128 个文件。
无语义、无 API、无 schema 变更，也不改测试断言。

## 变更

- 主要形态：导入排序（`{Value, json}` → `{json, Value}`）、长表达式/函数调用
  换行、`assert!`/`assert_eq!` 嵌套缩进、模块声明顺序按名称排序。
- 首批仅重排 119 个纯格式文件；本次其他 topic 的 8 个语义文件（store v41、
  session 重建）与遗漏的 `crates/llm/src/sse.rs` 在后续 `style:` 补齐提交中
  以同一 rustfmt 格式化，合计 128 个文件。

## 验证

- 将 `HEAD` 导出到独立 worktree，以同一 rustfmt 格式化后逐文件比对：
  首批 119 个文件与工作区完全一致，确认改动仅为格式化。
- 补齐提交后 `cargo fmt --all -- --check`：已跟踪文件零差异。
- 测试范围：纯格式改动，不运行测试；未运行全量测试。
