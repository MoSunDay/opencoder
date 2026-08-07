Commit: 4335e7be90a130b03fda64b9d42326d60a4f501d

# `ts` 全局 tmux session 管理

## Context

旧实现存在三处割裂：tmux 内执行裸 `ts` 会退化为 inline TUI；`ts -l` 虽全局展示，但停止项没有可恢复的 workdir；`ts -r`/`ts -c` 仍依赖当前目录或局部 store。结果是 session 离开 live tmux 后无法真正从全局面板管理。

## Change Summary

- 裸 `ts` 在 tmux 内外都创建新的 `opencode-<ulid>` session；tmux 内先 detached-create，再 `switch-client` 到新 session。
- 每个 ts store 旁原子写入 canonical `workdir` marker，不新增数据库表、字段或环境变量。
- 全局 registry 分页扫描所有 workdir store；live session 会回填 marker，单库超过 500 条时不截断。
- `ts -l` 为停止项展示已记录 workdir；`ts -r <id>` 从任意目录定位 store，并在原 workdir 冷启动。
- `ts -c` 全局保护 live tmux id，仅删除停止的 ts-owned seed；普通 `tui`/`run` 历史不删除，扫描或删除失败直接报错。
- `ts -l` 展示的 8 字符 ID 是可操作的唯一前缀；`ts -r` 与 `ts -d` 会在 live tmux 和全部 store 中还原完整 ID，前缀歧义时直接拒绝操作。
- `ts -l` 先按 workdir 升序分组，再在每个 workdir 内按创建时间倒序排列；live/stopped 状态不再拆散同一 workdir。
- `ts -d <id>` / `--delete <id>` 精确移除一个全局 managed session，支持列表 ID 前缀、完整 bare id、`opencode-<id>` 与 live tmux `$index`。非当前 live session 先 kill 再删唯一 Store 记录；当前 tmux session 和跨 Store 重复 ID 会拒绝删除，避免半途终止或扩大删除范围。

## Impact Surface

- `opencoder ts` / `rs` 的创建、列表、恢复、批量清理和精确删除
- tmux 内命令分发、per-workdir store 元数据与跨目录恢复

## Notes / Compatibility

历史停止 session 若从未处于本版本可见的 live tmux 中，首次全局恢复需要提供 `--workdir <original-path>`；验证归属后会补写 marker，此后可在任意目录恢复。无 schema、表或环境变量变更。

## Related Docs

- [CLI 模块](../../../agents/cli/index.md)
