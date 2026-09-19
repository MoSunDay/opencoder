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

## 测试契约
- bash 工具 spawn 为 `bash -lc` 登录 shell，会 source 宿主 profile；后台输出 8MiB 上限测试通过 `ToolContext.extra_env` 自带独立 `HOME`，不依赖宿主环境。
- 集成测试套件隐含契约：需具备 `HOME`/`SHELL` 的登录式环境（`tests/bash_guard_plan_mode.rs` 显式断言 `$HOME set`）；CI/门禁最小环境须导出 `HOME SHELL USER LOGNAME TERM`。
