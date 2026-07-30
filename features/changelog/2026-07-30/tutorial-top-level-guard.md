# fix(tui): 子代理视图不再显示新手教程

## 背景

体内新手教程（empty-session tutorial）在 chat 无任何 block 时渲染。当用户切到一个
**空的子代理（child）视图**时，该教程也会出现——但教程（"OpenCoder" 引导文案）仅对
顶层会话有意义，出现在子代理视图里既误导又突兀。

## 变更

`render_body`（`crates/tui/src/render.rs`）新增 `is_top_level: bool` 形参，教程守卫由

```rust
if chat.blocks.is_empty() { ... }
```

收紧为

```rust
if is_top_level && chat.blocks.is_empty() { ... }
```

`is_top_level` 由 `frame.rs` 的渲染管线传入：顶层会话为 `true`，子代理（child）视图为
`false`。顶层空会话行为不变；空子代理视图改为显示纯净的空 transcript 区块。

## 影响

- 仅影响空子代理视图的渲染分支，顶层会话与已有内容的视图零变化。
- 纯渲染逻辑，无状态/持久化/执行路径改动。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 空 child 视图（is_top_level=false）不渲染教程文案 | `empty_child_view_does_not_show_tutorial` | `crates/tui/src/render_tests/body.rs`（unit，纯渲染） |


## 验证

- `cargo test -p opencoder-tui empty_child_view_does_not_show_tutorial` -> 1 passed。
- `cargo test --workspace --all-targets` -> 全绿，0 failed。
