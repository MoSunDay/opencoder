Commit: (working-tree, pre-initial-commit)

# `/model` slash 命令：运行时配置模型 / 思考深度 / base_url / api_key / 上下文阈值

## Context
此前模型与 provider 配置只能在启动前写进 opencode.json 或设环境变量，TUI 启动后即固定（model 仅在状态栏显示，base_url/api_key 不可改）。思考深度（reasoning）只有入站 `reasoning_content` 解析与展示，**出站请求体没有任何 reasoning_effort / thinking 字段**——无法主动要求模型思考。用户希望在 TUI 里用 `/model` 随时改这些值并即时生效。

## Change Summary
- **core：Config 增 `reasoning_effort: Option<String>`**（`crates/core/src/config.rs`，`low|medium|high`，`None` 省略字段）；`merge_into` 解析之；新增 `Config::save(workdir, patch)` / `save_target(workdir)`（项目优先、全局兜底：首个含可编辑键的候选文件，无则在工作目录根创建 `opencode.json`）+ `looks_like_env_var` 判定纯大写 `_` 串（决定 api_key 是否包成 `{NAME}` 引用）+ `merge_json` 深度合并（保留无关键）。
- **llm：`ChatRequest.reasoning_effort`**（`crates/llm/src/request.rs`）：`to_body` 在非空时发顶层 `reasoning_effort` 字段，空白/None 省略。
- **session：runner 主调用读取 `session.config.reasoning_effort`** 透传进 `ChatRequest`；compaction/resume 的后台调用显式置 `None`（后台摘要无需思考）。
- **tui：slash 命令注册器**（`crates/tui/src/command.rs`，新增，286 行）：`COMMANDS` 注册表（`/task`、`/model`）+ `parse()` + `CommandMenu`（过滤/↑↓/Enter/Esc）+ `render_command_popup`。空 composer 输入 `/` 弹出选择器（取代旧「`/` 直接开 task picker」；裸 `/`+Enter 默认 `/task` 保留肌肉记忆）。
- **tui：`/model` 模态**（`crates/tui/src/model_menu/`，新增，拆 state.rs 361 + view.rs 75 + mod.rs 99）：5 字段表单（model / base_url / api_key 掩码 / reasoning 四档循环 / context_threshold）。api_key 显示 `sk-****1234`（首 2 末 4），编辑不回显（asterisks），未编辑保存时 `api_key=None` 保留原值。Tab/Shift+Tab 移动焦点，Enter on [Save] 校验后提交。
- **tui：热重载**：保存 → `Config::save` → `Config::load` → `UiCmd::ReloadConfig(Config)`；worker（主 worker 与 `/task` 切换 worker 两处）在收到时经 `SessionState::apply_config_reload(cfg, new_client)` 原地替换 `client/model/config`，于 turn 边界生效。**外层 `client` 绑定亦同步重建**（`run_app` 的 `mut client`），故 `/task` 新建会话拿到最新 endpoint（修复 stale-client：否则新会话沿用启动时的旧 client）。状态栏 model 旁显示 `·high` 徽标；keybind help 更新 `/` 说明；CLI `opencode models` 增 `thinking` 行。
- **session：`apply_config_reload`**（`src/lib.rs`，新增方法）：`apply_config_reload(cfg, Arc<dyn ChatStream>)` 原地替换三字段；调用方负责构建新 client，session 不耦合 `ChatClient` 具体类型，可经 `MockChatClient` 测试。

## Impact Surface
- 新增文件：`crates/tui/src/command.rs`、`crates/tui/src/model_menu/{mod,state,view}.rs`、`crates/llm/tests/request_body.rs`（3 测试）、`crates/session/tests/config_reload.rs`（3 测试：热重载字段/路由替换、`/task` 新会话用新 client 回归、同 client 仅换 config）。
- 修改：`crates/core/src/{config.rs,lib.rs}`、`crates/llm/src/request.rs`、`crates/llm/tests/mock_contract.rs`、`crates/session/src/{lib.rs(apply_config_reload),runner,compaction,resume}.rs`、`crates/tui/src/{app.rs,render.rs,keybind.rs,lib.rs}`、`crates/cli/src/session_cmd.rs`、`crates/core/tests/config_contract.rs`（+7 测试，含 HOME 隔离 helper、`null` 删 reasoning_effort、`{ENV}` api_key 往返）。
- 行为契约：`/` 由「直接开 task picker」改为「开 slash 命令选择器」（裸 `/`+Enter 仍 = task）。base_url/api_key 改动经 worker 热重载即时生效，无需重启；`/task` 新会话也用最新 endpoint（不再 stale）。api_key 若输入纯大写 `_` 串（如 `ZHIPU_API_KEY`）会包成 `{ZHIPU_API_KEY}` 写盘，复用现有 `{VAR}` 环境变量解析；未编辑的 api_key 保存时既不覆盖也不删除。

## Notes / Compatibility
- 思考深度协议选 OpenAI 风格 `reasoning_effort`（user 选定），glm-4.5/4.6 与 OpenAI o-series 兼容；未做 provider 路由分派。
- `/model` 编辑 api_key 为掩码不回显，规避 TUI 明文泄露；落盘值由 `looks_like_env_var` 决定明文或 `{ENV}` 引用。
- `Config::save_target` 的候选顺序与 `config_candidates` 一致（project-first）；创建新文件固定落到 `<workdir>/opencode.json`。
- 测试需隔离 `HOME`+`XDG_CONFIG_HOME`，否则 `Config::load` 会拾取开发者真实全局配置（`reasoning_effort_defaults_to_none` 测试已加 `isolated_home` guard）。
- **e2e（`scripts/e2e-glm.sh`）**：`reasoning_effort` 用 `medium`（非 high）以降低 live turn 内存/时长，避免被 sandbox reap；E7（`opencode models` 显示检查）**前置到 E1 之前**（廉价、无模型调用），故即使重型 live 步骤被杀，显示路径仍先验证。E1 的 run.log 重定向已补（修复 E2 session-id 比对的预存脆弱性）。**全量 e2e 已通过：10 passed / 0 failed**（E7 显示 → E1 snake → E2 resume+scoreboard → E6 thunder-fighter）。
- **CLI 显示路径单测**：`models_dispatch` 抽出 `models_summary(&Config) -> String`，新增 2 测试（reasoning_effort set/unset），不再依赖 smoke 或真模型即可覆盖。
