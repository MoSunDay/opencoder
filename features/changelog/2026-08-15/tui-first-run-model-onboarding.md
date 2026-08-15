Commit: 2a89df3e3c01cc1e928b8eb52c71d22105172ebb

# TUI 首次启动模型配置引导

## Context

此前交互式启动会在缺少 API key 或端点配置时直接报错退出。新用户必须先离开程序、手工创建并编辑配置文件，完成后才能开始任务，首用链路不闭环。

## Change Summary

- 交互式 TUI 启动会以 never-clobber 方式确保 `~/.opencoder/config.json` 存在；Unix 新文件权限为 `0600`，取消引导时保留合法空 JSON，便于下次继续。
- 对 global、project、CLI 与环境变量合并后的有效配置执行本地就绪校验；已有可用配置直接进入任务界面。
- 配置不可用时，在同一个 TUI alt-screen 内复用 Provider 表单，引导填写 provider、model、base URL、API key 与 headers；预填有效值并聚焦密钥字段。
- 保存前验证凭证、HTTP(S) URL、headers 与代理，并构建 `ChatClient`，但不发送模型探测请求。保存固定写入全局配置，完整重载覆盖层后再次验证，成功即进入任务 composer。
- Esc/Ctrl-D 可直接退出，不创建 Session；终端和 tmux 状态由 RAII 在正常退出与错误路径统一恢复。
- `Config` 新增精确全局路径、never-clobber 创建、全局保存与纯函数 merge API；日常 `/model` 的项目优先保存语义保持不变。

## Impact Surface

- `crates/core/src/config.rs`、`crates/core/src/config/env.rs`
- `crates/tui/src/onboarding.rs`、`crates/tui/src/app_bootstrap.rs`
- `crates/tui/src/model_menu/provider_form.rs`

## Validation

- Core 配置合同新增 4 项：空文件创建且不覆盖、Unix 私有权限、全局保存不受项目目标影响且保留未知键、纯函数 merge。
- TUI onboarding 新增 8 项：缺密钥/非法 URL、无网络就绪、表单预填与聚焦、Esc/Ctrl-D、patch、保存后重载、项目覆盖冲突、密钥掩码渲染。
- 真实 PTY：空 HOME 首次显示引导并创建 `0600` 配置；取消后第二次启动继续引导；提供有效 `OPENAI_API_KEY` 时跳过引导并直接进入 composer。
- `cargo test --workspace`：2573 passed / 0 failed（基线 2561 + 本轮新增 12）。
- `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo build --workspace` 通过。
