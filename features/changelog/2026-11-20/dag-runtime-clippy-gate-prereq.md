# 修复 dag-runtime 预存 `needless_range_loop`，解除相关 crate 的 Clippy 门禁阻塞

一句话：`runtime/dynamic.rs` 恢复扫描把 `for i in 0..len` + `group.outcomes[i]`
下标读改为 `iter().enumerate()`，行为不变；该预存 lint 在
`cargo clippy -p <workspace-crate> --all-targets -- -D warnings` 时随路径依赖
一起编译报错，阻塞了本次四个稳定性修复（hub 超时回收、persistence 自愈、
SSE 硬上限、SSE 有界回放）的规则 02 Clippy 门禁。

## 背景与结论

- `-D warnings` 经 RUSTFLAGS 传播到同 workspace 路径依赖，`opencoder-dag-runtime`
  的这一条 warning 使任何依赖它的 crate 无法以零警告通过门禁。
- 仅改写循环形式：`group.outcomes` 与 `group.items` 等长（构造处
  `vec![None; items.len()]`），enumerate 语义等价，无行为变化。

## 变更清单

- `crates/dag-runtime/src/runtime/dynamic.rs` — 恢复扫描循环去下标读。

## 测试覆盖

无新增测试：行为保持不变（纯 lint 改写）。

- Clippy：`cargo clippy -p opencoder-dag-runtime --all-targets -- -D warnings` →
  零警告；`cargo clippy -p opencoder-worker --all-targets -- -D warnings` →
  零警告（此前被本条阻塞）。
- 测试范围：仅本条 lint 改写及其门禁验证；未运行全量测试。
