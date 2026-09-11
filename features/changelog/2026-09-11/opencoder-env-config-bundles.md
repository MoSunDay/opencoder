Commit: 75f916e8564fd53031e59a01b5572d8df3cb9a61

# Web Env 管理对齐 OpenCoder 配置集

## Context

Web 的 Env 页面此前使用任务系统的 `/api/todo/envs` context，只能维护环境变量和工具引用，无法表达 OpenCoder 的完整环境配置。

## Change Summary

控制面新增 `/api/envs` 配置集接口，直接管理 `~/.opencoder/envs/<name>/` 下的 `config.json`、`mcp.json`、`cli.json`、`skills.json` 与 `ap.json`，支持列表、激活/停用、新建、重快照、编辑和删除。前端 Env 抽屉按文件编辑并一次保存整套配置；激活环境的保存会触发运行时重新加载。读取配置会遮蔽 `api_key`，提交遮蔽值时保留原凭据。

## Impact Surface

- 控制面路由：`/api/envs`、`/api/envs/:name`、`/api/envs/:name/recapture`
- Web SPA：Env 管理页面切换到 OpenCoder Env 语义，可切换 opencoder.json、Skill、CLI 等配置
- 核心配置层：暴露 Env 文件的 owner-only 写入包装，沿用现有 0o600 约束

## Notes / Compatibility

任务系统的 `/api/todo/envs` 保持不变，仍供 TODO 模板工具绑定使用；两者不再混用。生产发布包为 `opencoder-platform-75f916e8`。

## Validation

- `cargo check -p opencoder-server`
- `cargo clippy -p opencoder-control --all-targets -- -D warnings`
- `cargo test -p opencoder-web --test web_envs`（11 passed）
- `npm run build`（SPA drift check passed）
- 远端 `/usr/local/bin/opencoder-server --build-info` 与 agent 均报告 commit `75f916e8`、`git_dirty=false`
