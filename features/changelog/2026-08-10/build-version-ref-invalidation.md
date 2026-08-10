Commit: 76a0d8b9de6e8da492f689e2448eb4a2c12213ec

# Git 分支提交后刷新构建版本信息

## Context

`opencoder --version` 的 commit id 在构建期写入。既有 `build.rs` 只监听 `.git/HEAD` 与 `packed-refs`，但普通分支的新提交只推进 `.git/refs/heads/<branch>`，HEAD 文件内容不变；因此提交后增量 release 构建可能继续安装旧 commit 和 `-dirty` 标记。

## Change Summary

- `rerun_for_git` 增加对 `.git/refs` 的递归变更监听，覆盖 loose branch/tag refs。
- 保留 HEAD 与 packed refs 监听，继续覆盖分支切换和 refs 打包。
- 不改变版本格式、公开接口、配置或运行时行为。

## Validation

- 在 dirty 工作树构建后提交，再次执行增量 release 构建，版本由旧的 `de4e210-dirty` 自动刷新为新提交 id，且不再带 dirty 标记。
- `cargo test --workspace` 与 workspace clippy 通过。

## Related Docs

- [版本信息带上 git commit id](../2026-08-04/version-carries-commit-id.md)
