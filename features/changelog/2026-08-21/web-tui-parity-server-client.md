# feat(web,client,cli,session,core): Web 能力 100% 对齐 TUI + Server/Client 模式 100% 可用

## 背景

Web 侧三层（HTTP API / 内嵌前端 / `opencode client`）与 TUI 的会话能力存在系统性差距：

- **question 交互断链**：plan agent 的 `question` 工具在 web drain 下无监听者，
  固定回 `NO_LISTENER` 兜底文案——用户永远没机会作答；`QuestionHub` 只广播
  call_id，payload（问题文本/选项）不出 hub，web 端即便轮询也拿不到要显示什么。
- **无 queue/steer 管理**：steer/queue 只能投递（`POST /prompt` 的 `delivery`
  字段），不能查看、删除、重排——TUI 侧 pending 输入可见可管理，web 侧是黑盒。
- **无 annotation / autopilot / plan 相位端点**：requirement 标注、会话级
  autopilot 三态、切回 plan 时的相位计数清零，TUI 都有对应操作，web 无口子。
- **无模型/技能发现**：前端模型下拉框是硬编码列表；`opencode client` 无法
  枚举 server 配置了哪些模型/技能。
- **title 截断非 LLM**：web 创建的会话标题是首条 prompt 截断，TUI/headless 走
  small_model 异步生成。
- **client 半残**：`Remote` 缺 18 个方法（fork/compact/handoff/skill/annotation/
  autopilot/models/skills/questions/inputs/config 等）；SSE 断流即死（无重连），
  `interrupt` 返回裸 `()` 无法区分成败。
- 前端 `render.js`/`app.js` 两个大杂烩文件（各自 200+ 行无边界），SSE 无重连，
  question/queue 面板不存在。

## 变更

### Phase 0 — session 使能（`crates/session/src/tools/question.rs`）

- `QuestionHub` 携带问题载荷：`pub struct QuestionPayload { question, options }`；
  新增 `ask_with_payload()`（提问时连同 payload 一起入 hub）与
  `waiting_questions() -> Vec<(id, payload)>`（按 id 枚举待答问题）；
  `abandon()` 同步丢弃 payload；工具 execute 传入真实 question+options
  （此前 hub 只见 call_id）。

### Phase 1 — web server

- `crates/core/src/data_dir.rs`：新 `pub fn workdir_hash(&Path) -> String`
  （canonical 化路径的十六进制摘要，与 `data_dir_for` 共享同一算法——列表过滤与
  数据目录选址永不漂移）。
- `crates/web/src/cmd.rs`：`DrainCmd` 增 `SetApMode(ApMode)` /
  `SetAnnotation(Option<String>)` / `ResetPlanPhase`（镜像 TUI worker.rs 语义）。
- `crates/web/src/handle.rs` + `handle_questions.rs`（新）：
  - `SessionHandle.question_hub`：`Arc<QuestionHub>` 在 handle 上跨 drain 稳定
    存活（drain/resume 重建 session 时 rebind `session.question_hub` 并 attach），
    question 工具从此可在 web 下等待作答。
  - 最后一个 SSE 订阅者断开 → abandon 全部 waiting questions（工具得 SKIPPED）。
  - 首次 drain run 成功后 best-effort LLM 标题生成（30s 超时；已有 title 则跳过）。
- `crates/web/src/api_questions.rs`（新）：`GET /api/sessions/:id/questions`
  （轮询即 attach——poll 本身算作监听）、`POST .../questions/:call_id/answer`、
  `POST .../questions/:call_id/skip`。
- `crates/web/src/api_inputs.rs`（新）：`GET /inputs?delivery=queue|steer`、
  `DELETE /inputs/:seq`、`POST /inputs/reorder`。
- `crates/web/src/api_meta.rs`（新）：`POST /annotation`（设置/清空 requirement）、
  `POST /autopilot`（off|ap|review|null=清除 override）、`GET /api/models`
  （脱敏：永不返回 api_key/headers；去重后的下拉列表）、`GET /api/skills`
  （仅 name/description/enabled）。
- `crates/web/src/api.rs`：`list_sessions` 增 `?workdir=` 过滤（经
  `workdir_hash`，新会话行打戳）；`get_events` 增 `Last-Event-ID` header 回退；
  `post_agent` 切到 plan 时 `ResetPlanPhase` 并持久化 `plan_input_count=0`。
- `crates/web/src/lib.rs`：注册 10 条新路由。

### Phase 2 — client + CLI

- `crates/client/src/remote_ops.rs`（新）：18 个 `Remote` 方法（get/delete/fork
  session、compact、handoff、skill、annotation、autopilot、models、skills、
  config get/patch、questions list/answer/skip、inputs list/delete/reorder）。
- `crates/client/src/remote.rs`：`list_sessions(limit, search, workdir)`；
  `post_prompt` 增 skill 参数；`interrupt` 返回结构化 `{ok,error}` Value；
  SSE 合成失败 kind 改 `stream_error`（与 LLM 流式错误区分）。
- `crates/cli/src/client_stream.rs`（新）：纯函数重连策略（3 次重连，
  500ms/1s/2s 退避）；`stream_with_reconnect`（业务错误即终止；传输层断开 →
  从 `/seq` 快照游标重新订阅；放弃重连 → transcript 快照兜底 + 报错）；
  question 工具 stderr 提示（含 call_id，引导用子命令作答）。
- `crates/cli/src/client_ops.rs`（新）：`client session list|show|delete|fork`、
  `client questions list|answer|skip`。
- `crates/cli/src/client.rs`：`ClientRunOpts` 编排——fork → autopilot →
  annotation → interrupt/compact/handoff（终态操作）→ steer-task/prompt →
  stream；`--continue` 按 workdir 过滤（不再跨 workdir 误续）。
- `crates/cli/src/lib.rs`：client 旗标 `--delivery` / `--skill`（可重复，最后一个
  胜出）/ `--fork` / `--compact` / `--handoff [extra]` / `--autopilot` /
  `--annotation` / `--steer-task` / `--workdir` + `ClientSub` 子命令树。
- `src/main.rs`：分发接线。

### Phase 3 — web 前端模块化（assets 全部 ≤400 行）

- 新模块：`api.js`（fetch+token）、`sse.js`（EventSource + 重连：经 `/seq` +
  `?after=` 续订，5 次尝试退避 1..16s，badge+banner）、`sessions.js`
  （agent/model/skill-NAME 徽标修复、相对时间、防抖服务端搜索）、`chat.js`
  （transcript 管线 + llm_usage chips + subagent 卡片嵌套 child delta +
  steer-consumed chips + plan 卡片 + autopilot chip + 空会话欢迎页）、
  `composer.js`（steer/queue 键、图片、`$skill` 弹窗 → body.skill、subagent
  内联 steer）、`questions.js`（1.5s 轮询卡片：选项按钮/自由文本/跳过、乐观
  resolve）、`queue_panel.js`（pending 输入抽屉：删除/重排、badge 计数）、
  `settings.js`（模型下拉来自 /api/models + 自定义回退、autopilot 选择、
  annotation 编辑、handoff/fork/compact）。
- `index.html` 重构（8 个 script 标记）、`styles.css` 扩展；`render.js`/`app.js`
  删除（职责被吸收）；`html.rs` 按依赖序内联全部模块。

## 语义要点

1. **question hub attach 语义**：drain 时 attach（resume 后 rebind）；最后一个
   SSE 订阅者断开时 abandon 全部待答问题（工具得 SKIPPED）；attach 是**粘性**
   （无 detach）——多客户端同在线时只有最后一个离开才触发 abandon，先离开者
   不影响其它在线者的作答能力。
2. **SSE live 事件无 seq**：重连窗口内可能重复/漏掉少量事件——已知限制，不做
   落库管线重构；兜底是 `/seq` 快照 + messages 重建（client 侧重连、前端重连
   同走此路径）。
3. **workdir 过滤两端一致**：`?workdir=` 过滤要求 client 与 server 看到相同路径
   （默认 client cwd）；不匹配返回空列表，需显式 `--workdir`。新会话打
   `workdir_hash` 戳；**旧 NULL-hash 会话不被过滤匹配**（无戳即不可见于按
   workdir 过滤的结果集）。
4. **client 首词子命令冲突**：`opencode client <prompt>` 的 prompt 首词为
   `session` / `questions` 时会被解析为子命令——用 `--` 分隔符规避
   （`opencode client -- session ...`）。
5. **generate_title 默认开启**：web drain 首次成功 run 后 best-effort 生成
   （30s 超时）；失败非致命（保留截断标题）。
6. **models 端点脱敏**：`GET /api/models` 永不返回 api_key/headers（构建模型
   客户端所需之外的凭证字段一律剔除）。
7. **排除项（明确不对齐）**：notepad/vim、copy mode、快捷键改绑、`!cmd`、
   TODO 工作流、TUI EditPlan（requirement 端点覆盖其主要用途）；**不加 CORS**。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| hub 枚举待答 payload | `waiting_questions_lists_payloads_by_id` | `crates/session/src/tools/question.rs` |
| abandon 丢 payload | `abandon_drops_the_payload_too` | `crates/session/src/tools/question.rs` |
| 早答仍广播 payload 至消费 | `early_answer_still_publishes_payload_until_consumed` | `crates/session/src/tools/question.rs` |
| workdir_hash 与 data_dir 同源 | `workdir_hash_matches_data_dir_for_component` | `crates/core/src/data_dir.rs` |
| workdir_hash canonical+稳定 hex | `workdir_hash_canonicalizes_and_is_stable_hex` | `crates/core/src/data_dir.rs` |
| question 作答闭环（tool result 同轮回填） | `answer_flows_into_tool_result_and_completes_the_turn` | `crates/web/tests/web_questions.rs` |
| skip → SKIPPED | `skip_resolves_tool_to_skipped_reply` | `crates/web/tests/web_questions.rs` |
| 未知 call_id / 空 answer → 400/404 | `unknown_call_id_and_empty_answer_are_rejected` | `crates/web/tests/web_questions.rs` |
| 空问题列表 200 数组 | `list_questions_empty_is_200_with_array` | `crates/web/tests/web_questions.rs` |
| 最后订阅者断开 → abandon | `last_subscriber_disconnect_abandons_waiting_question` | `crates/web/tests/web_questions.rs` |
| queue 列表/删除/重排 | `queue_inputs_list_delete_and_reorder_while_drain_hangs` | `crates/web/tests/web_inputs.rs` |
| inputs 默认 steer + 未知会话容忍 | `list_inputs_defaults_to_steer_and_tolerates_unknown_session` | `crates/web/tests/web_inputs.rs` |
| annotation 设置/清空/404 | `annotation_set_clear_and_missing_session` | `crates/web/tests/web_meta_endpoints.rs` |
| autopilot ap/review/非法/清除 | `autopilot_set_invalid_and_clear` | `crates/web/tests/web_meta_endpoints.rs` |
| models 脱敏+按 provider | `models_endpoint_is_sanitized_and_lists_providers` | `crates/web/tests/web_meta_endpoints.rs` |
| models 下拉去重（default a/b + provider a/b 恰一次） | `models_endpoint_dedupes_default_and_named_provider_ids` | `crates/web/tests/web_meta_endpoints.rs` |
| skills 形状（name/desc/enabled） | `skills_endpoint_returns_name_description_enabled` | `crates/web/tests/web_meta_endpoints.rs` |
| 会话列表 workdir 过滤 | `session_list_filters_by_workdir_hash` | `crates/web/tests/web_list_events.rs` |
| Last-Event-ID header 驱动 replay | `last_event_id_header_drives_replay` | `crates/web/tests/web_list_events.rs` |
| SetApMode/SetAnnotation 应用到 live drain | `autopilot_and_annotation_cmds_apply_to_live_drain` | `crates/web/tests/web_drain_cmds.rs` |
| ResetPlanPhase 持久化计数器 | `reset_plan_phase_cmd_persists_zero_counter` | `crates/web/tests/web_drain_cmds.rs` |
| 切 plan 持久化 plan_input_count=0 | `agent_switch_to_plan_persists_zero_plan_input_count` | `crates/web/tests/web_drain_cmds.rs` |
| client fork→delete 往返+404 | `fork_then_delete_session_roundtrip` | `crates/web/tests/client_remote_ops.rs` |
| client compact/handoff/skill 受理 | `compact_handoff_and_skill_are_accepted` | `crates/web/tests/client_remote_ops.rs` |
| client config GET 脱敏对象 | `config_get_returns_redacted_object` | `crates/web/tests/client_remote_ops.rs` |
| client questions 空列表/未知 answer 404 | `questions_list_empty_and_unknown_answer_404` | `crates/web/tests/client_remote_ops.rs` |
| client inputs 列表+delivery 校验 | `inputs_list_and_delivery_validation` | `crates/web/tests/client_remote_ops.rs` |
| client annotation/autopilot 往返+400/404 | `annotation_and_autopilot_roundtrip` | `crates/web/tests/client_remote_ops.rs` |
| client models/skills 目录对象 | `models_and_skills_catalogs_are_objects` | `crates/web/tests/client_remote_ops.rs` |
| SPA 标记替换+资产内联 | `markers_replaced_and_assets_inlined` | `crates/web/src/html.rs` |
| script 哨兵按依赖序 | `script_sentinels_present_in_dependency_order` | `crates/web/src/html.rs` |
| 无 module/外部引用 | `no_module_or_external_references` | `crates/web/src/html.rs` |
| client 新旗标解析 | `client_new_flags_parse` | `crates/cli/tests/cli_parse.rs` |
| delivery 默认 steer | `client_delivery_defaults_to_steer` | `crates/cli/tests/cli_parse.rs` |
| handoff 有/无 extra | `client_handoff_parses_with_and_without_extra` | `crates/cli/tests/cli_parse.rs` |
| 未知 delivery/autopilot 仍解析（运行时校验） | `client_unknown_delivery_and_autopilot_values_still_parse` | `crates/cli/tests/cli_parse.rs` |
| session 子命令树 | `client_session_subcommands_parse` | `crates/cli/tests/cli_parse.rs` |
| questions 子命令树 | `client_questions_subcommands_parse` | `crates/cli/tests/cli_parse.rs` |
| 首词形如子命令时子命令胜出 | `client_subcommand_wins_over_prompt_shaped_text` | `crates/cli/tests/cli_parse.rs` |
| 重连退避 500ms/1s/2s 后放弃 | `reconnects_with_exponential_backoff_then_gives_up` | `crates/cli/src/client_stream.rs` |
| workdir 解析（flag > cwd） | `workdir_prefers_global_flag_then_cwd` | `crates/cli/src/client.rs` |
| autopilot 合法值/拒绝其它 | `autopilot_accepts_known_modes_and_rejects_others` | `crates/cli/src/client.rs` |
| Client 旗标回退全局（session/continue） | `client_session_flags_fall_back_to_globals` / `client_continue_flags_or_with_globals` | `crates/cli/src/client.rs` |
| ClientOpts 纯数据全旗标冒烟 | `client_opts_is_plain_data` | `crates/cli/src/client.rs` |
| generate_title 成功 drain 落库（小模型请求） | `successful_drain_persists_generated_title` | `crates/web/tests/web_title.rs` |
| 已有 title 跳过生成（不覆盖/不加轮） | `existing_title_skips_generation` | `crates/web/tests/web_title.rs` |
| 前端 headless 运行时冒烟（问答/队列/下拉/重连/发送 27 断言） | `frontend_headless_smoke`（node: `tests/frontend_smoke.mjs`） | `crates/web/tests/web_frontend_runtime.rs` |
| 真实二进制 server+client 全旗标矩阵 | `client_server_flag_matrix_smoke` | `tests/client_server_smoke.rs` |

- 全量回归：`cargo test --workspace` → **3250 passed / 0 failed**（215 个测试
  二进制；评审修复后复跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- fmt：`cargo fmt --check` → 干净

## 回归门（rules/02）

- `cargo fmt --check` → ✅
- `cargo clippy --workspace --all-targets -- -D warnings` → ✅ 零警告
- `cargo test --workspace` → ✅ **3250 passed / 0 failed**（215 二进制；含评审
  修复新增：web_title 2、models 去重 1、前端冒烟 1、真实二进制冒烟 1）
- 行数 gate：新增文件全部 ≤400 行；`crates/web/src/api.rs` 已拆出 SSE 端点至
  `api_events.rs`（159 行），api.rs 降至 676 行（评审修复）。
- 冒烟：真实二进制全旗标矩阵已固化为 `tests/client_server_smoke.rs`
  （起真实 server → 401 stub LLM → client 全旗标断言），随 cargo test 回归。

## 评审修复（web-tui-parity 上线前 review 跟进）

评审结论 85%（12/14），三项 P0 缺口 + 两项 P1 全部补齐：

1. **[P0] generate_title 专测**：新增 `crates/web/tests/web_title.rs`——成功
   drain 后小模型调用（`requests().last().model == small_model`）把 Completed
   文本落库为 title；已有 title 的会话跳过生成（默认脚本返回哨兵 title 证明
   未加轮、原 title 未被覆盖）。
2. **[P0] models 去重专测**：default `a/b` + provider `a` model `b` 时下拉数组
   中 `a/b` 恰出现一次且居首——补齐上表原"脱敏+去重"行缺失的去重断言。
3. **[P0] 前端运行时验收**：无浏览器环境，改走 headless 冒烟——
   `crates/web/tests/frontend_smoke.mjs`（309 行）在 node vm 中加载**真实**
   assets 八模块（DOM shim + mock fetch/EventSource），断言 27 项运行时行为：
   问答卡片渲染/作答/skip 闭环、队列面板列表（steer 优先）/重排/删除/qcount、
   模型下拉（目录+custom 回退）、composer 发送（乐观回显+busy 切换）、SSE
   重连徽标（出错显示、`?after=<seq>` 续流、事件到达复位、5 次失败出持久横幅）。
   由 `web_frontend_runtime.rs` 包一层（无 node 时 skip 并提示 NODE_BIN）。
4. **[P1] CLI 冒烟脚本化**：新增根包 `tests/client_server_smoke.rs`——起真实
   `opencode server`（--port 0 解析回显 URL），本地 401 stub 充当 LLM（非可重试
   → 立即失败），断言：空列表、错 token 401、--autopilot 非法值客户端拦截、
   --interrupt 无会话报错、真实 prompt run（错误呈现 401 且会话行落库存活）、
   show JSON、--annotation/--autopilot 配置即退、interrupt 结构化失败反馈、
   questions list/answer-404、fork→两行→delete 级联→清空。
5. **[P1] api.rs 拆分**：SSE 端点（`get_events`/`get_event_seq`/`EventsQuery`）
   迁至 `crates/web/src/api_events.rs` 并自 api.rs re-export（调用路径不变），
   api.rs 799 → 676 行。

复跑回归门：fmt ✅ / clippy -D warnings ✅ / `cargo test --workspace`
**3250 passed / 0 failed**（215 二进制）。
