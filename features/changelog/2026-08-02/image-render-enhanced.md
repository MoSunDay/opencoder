# TUI 内联图片渲染增强：宽度自适应 + Triangle 滤波 + HTTP 预取

## Summary

TUI 聊天流水中的内联图片渲染原先有三处不足：宽度写死、缩放锯齿严重、resume
时远端图片不渲染。本次按「Plan A」一并修掉——三项互相独立的增强：

1. **宽度自适应**（`image_render.rs` 的 `terminal_image_width()`）：通过
   `crossterm::terminal::size()` 查询实时终端宽度，减去开销（indent 4 + borders 2
   + text_w 减 1 = `IMAGE_WIDTH_OVERHEAD=8` cells），下限钳到 20；在 headless / 管道
   场景（查不到终端尺寸）回退到 120。供 `build_image_block`、`render_image_from_url`
   使用，并传给 `render_image_halfblock`。

2. **Triangle 滤波**（`image_render.rs` 的 `render_dynamic_image`）：缩小时改用
   `image::imageops::FilterType::Triangle`（替代默认的最近邻），让源图宽于可用
   终端宽度时下采样更平滑、锯齿更少。

3. **HTTP 预取**（`fetch_image_bytes` + `prefetch_image_bytes`）：新增异步函数
   `fetch_image_bytes(url)` 用一个经 `OnceLock` memoize 的 `reqwest::Client`
   拉 HTTP(S) 图片字节（10 秒超时）。`replay_into_chat`（现位于
   `session_ui/replay.rs`）在同步回放前先调 `prefetch_image_bytes` 收集全部
   HTTP(S) 图片 URL，使 resume 时远端图片也能渲染。`render_image_from_url`
   先查预取字节 map，再回退到同步 data-URI 解码。同步路径的
   `build_image_block` 对远端 URL 仍输出空（占位）。

> 注：重构（把 replay 函数从 `session_ui.rs` 抽到 `session_ui/replay.rs`）把
> `session_ui.rs` 从 846 行降到 471 行，满足 ≤800 的硬限制。

## Changes

### `crates/tui/src/image_render.rs`
- 新增 `terminal_image_width()`：实时终端宽度 - 开销，下限钳 ≥20，回退 120
- 新增常量 `IMAGE_WIDTH_OVERHEAD` = 8
- 新增 `fetch_image_bytes(url)`：异步拉取 data URI（进程内）与 HTTP(S)
  （reqwest + 10s 超时）
- 新增 `http_client()`：经 `OnceLock<reqwest::Client>` memoize
- 新增 `fetch_http_bytes(url)`：内部 HTTP GET 辅助
- 新增 `render_image_from_url(url, &prefetched)`：先查预取 map，回退 data-URI 解码
- `render_dynamic_image`：改用 `FilterType::Triangle`（替代默认）
- 新增 `width_tests` 模块，7 个测试

### `crates/tui/src/session_ui/replay.rs`（新建，从 `session_ui.rs` 抽出）
- `prefetch_image_bytes(messages)`：收集 HTTP(S) 图片 URL，异步拉取
- `replay_one`：新增 `&HashMap<String, Vec<u8>>` prefetched 参数，经
  `render_image_from_url` 渲染图片
- `replay_into_chat`：回放循环前先调 `prefetch_image_bytes`
- 从 `session_ui.rs` 抽出，使该文件保持在 800 行限制内

### `crates/tui/src/session_ui/image_prefetch_tests.rs`（新建）
- 4 个测试覆盖预取与回放交互

### `crates/tui/Cargo.toml`
- 新增 `reqwest` 依赖，开启 `rustls-tls` feature

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| terminal_image_width 下限 ≥20 | `terminal_image_width_returns_at_least_minimum` | `image_render.rs` (`width_tests`) |
| fetch_image_bytes 解码 data URI | `fetch_image_bytes_decodes_data_uri` | 同上 |
| fetch_image_bytes 未知 scheme 返 None | `fetch_image_bytes_returns_none_for_unknown_scheme` | 同上 |
| render_image_from_url 使用预取字节 | `render_image_from_url_uses_prefetched_bytes` | 同上 |
| render_image_from_url 回退 data URI | `render_image_from_url_falls_back_to_data_uri` | 同上 |
| render_image_from_url 无预取 HTTP 返空 | `render_image_from_url_empty_for_missing_http` | 同上 |
| Triangle 缩放大图 | `triangle_filter_renders_large_image` | 同上 |
| replay_one 预取 HTTP 图片渲染 | `replay_one_renders_prefetched_http_image` | `session_ui/image_prefetch_tests.rs` |
| replay_one 无预取 HTTP 图片占位 | `replay_one_http_image_without_prefetch_is_placeholder` | 同上 |
| replay_one 预取 tool 图片渲染 | `replay_one_prefetched_tool_image_renders` | 同上 |
| prefetch 跳过 data URI 仅收集 HTTP | `prefetch_skips_data_uris_and_collects_http` | 同上 |

## 全量回归

- 全量回归：`cargo test --workspace` → **1634 passed / 0 failed / 1 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`image_render.rs` 367（迭代 ≤800）；`session_ui/replay.rs` 388（新增 ≤400）；`session_ui.rs` 471（迭代 ≤800）
