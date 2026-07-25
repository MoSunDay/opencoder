# fix(tui/markdown): 多行代码块边框断裂 + 后续布局错乱

## 根因

`crates/tui/src/markdown.rs::flush_code()` 用 `trim_end_matches('\n')`
逐元素处理 `code_buf`。但 `pulldown-cmark` 0.13.4 把**整个** fenced 代码块
作为**单个** `Event::Text` 返回（内容含内嵌 `\n`，如 `"fn a() {}\nfn b() {}\n"`），
于是：

- `code_buf` 只有 1 个元素，循环只跑一次；
- `trim_end_matches('\n')` 只剥掉末尾一个 `\n`，**内嵌 `\n` 残留**；
- 多行代码被塞进**单个** `Line`，其 `Span` 文本含**字面量 `\n`**。

后果链（完全吻合症状）：终端遇字面量 `\n` 换行但续行无 `│ ` 边框 → **边框断裂**；
ratatui `Paragraph` 当作 1 行 → `line_count` 算错 → 滚动 / 可见行数 /
thinking 头部点击热区全部错位 → 后续内容堆叠错乱。旧测试 `code_block()` 只测
单行代码（`"fn main() {}"`），故潜伏至今，表现为「LLM 输出多行代码块时概率性
崩坏」。

## 变更

### `crates/tui/src/markdown.rs::flush_code()`（核心修复）
- 将 `code_buf` 各 chunk `concat()` 成单一字符串，按 `\n` 拆分为独立逻辑行，
  **每行渲染成独立 `Line`**——根因消除。
- 仅在「末尾恰好一个空串元素」时 `rows.pop()`（而非 `trim_end_matches`），
  保留有效的内部空行（空代码块则一条正文都不渲染）。
- 每行 `strip_suffix('\r')` 兼容 CRLF。
- 顺带把 `map_or(false, …)` 改为 `is_some_and(…)` 以过 clippy。

### `crates/tui/src/chat.rs:479`（顺带加固）
流式纯文本路径 `raw.split('\n')` 增加 `strip_suffix('\r')`，消除同类 CRLF
残留（影响小：turn done 后会重渲染，零状态变更）。

### `crates/tui/src/session_ui.rs:593`（unblock 编译）
补上 in-progress images 特性遗漏的 `images: Vec::new()` 字段（与同文件
~662/~668 两处构造一致），解除 tui crate 测试无法编译的阻塞。

## 测试清单（功能 → 测试名）

| 功能 | 测试 | 结果 |
| --- | --- | --- |
| 多行代码块每行独立 `│` 边框、无字面量 `\n` | `markdown::tests::multi_line_code_block` | ok |
| 含空行代码块的边界（空行也有独立 `│` 行） | `markdown::tests::code_block_with_blank_line` | ok |
| CRLF 输入不残留 `\r` / `\n` | `markdown::tests::crlf_code_block` | ok |
| 单行代码块回归（不退化） | `markdown::tests::code_block` | ok |

## 验证

- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` — 零警告
- `cargo test -p opencoder-tui markdown` — 12 passed / 0 failed
- `cargo test -p opencoder-tui` — 436 passed / 0 failed（全量回归）
