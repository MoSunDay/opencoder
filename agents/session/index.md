Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# session 模块

会话运行时：drain 循环、工具注册、subagent、plan 写拦截、压缩、resume、cancel。

## 索引
- `src/session.rs` — drain 主循环与 turn 流转
- `src/tools/` — 工具注册与实现
- `src/subagent.rs` — 子代理
- `src/compaction.rs` — 上下文压缩
- `src/resume.rs`、`src/cancel.rs` — 恢复与取消

## 接缝
- 只依赖 `Arc<dyn Store>` 与 `Arc<dyn ChatStream>`，不做 HTTP/终端 IO。
- steer 打断进行中 turn；queue 等 idle；均先 pending 落库。
