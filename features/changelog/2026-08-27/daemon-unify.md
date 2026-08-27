Commit: (working-tree, daemon 统一入口 + 全量签名 + SPA 内嵌 + 浏览器验收收口)

# daemon 统一入口 + 全量 HMAC 请求签名 + React/antd 舰队控制台内嵌（上线前评审收口）

## 背景

上线前评审的 7 项 TODO 一轮收口：

1. 旧进程冒烟改走 daemon 入口，全量回归 0 failed；
2. P4.6 SPA 接线——`html.rs` 改为编译期内嵌 `crates/web/spa/dist`（React18 + antd + Vite 构建产物以固定文件名入库），删除旧 vanilla 内联前端（`crates/web/src/assets/*` 13 文件）与 4 个 mjs/shim 前端测试；
3. 清理 unused import；
4. `cli_parse.rs` 434 行拆分过行数门；
5. clippy `-D warnings` + release build 双绿；
6. 进程级 e2e 冒烟实跑取证；
7. 本文档与 `agents.md` 逻辑地图同步。

前置项（评审时已有当次证据、本轮复核）：`crates/client` crate 删除；`server`/`client`/`node` 三子命令删除（不留别名，`daemon --server | --client` 唯一入口）——统一入口的动机是双入口/别名并存期冒烟脚本与文档各自漂移，单一分发点让角色语义（token 校验、remote 必填）只在一处收口；`core/src/auth_sig.rs` 签名协议；web 签名中间件；node uplink 单一签名出口 `signed_request`；P3 消息回拉 relay；register 记录来源 IP（schema v13 `nodes.last_addr`）。

## 实现

### cli（crates/cli）

- `src/daemon.rs`（191 行）：`daemon_mode` / `resolve_client_token` / `default_node_name` 纯函数 + 10 个单测（角色互斥、token 解析、节点名默认值唯一性全在此收口）。token 解析契约：flag > env、空 env 视同缺失、**永不自动生成**。
- 行数门拆分：`tests/cli_parse_daemon.rs` 新文件（111 行）承载 5 个 daemon 解析契约测试；`tests/cli_parse.rs` 434 → 329 行。新文件行数门 ≤400 全量核对通过；全仓迭代文件最大 796 行（`crates/tui/src/app.rs`）≤800。

### web（crates/web）

- `src/html.rs` 重写（139 行）：`include_str!` / `include_bytes!` 编译期内嵌 `../spa/dist`——`cargo build` 无需 node，二进制伺服的就是入库的那份 dist。
- 路由契约：`/` 伺服 SPA 壳；`/static/:name` 白名单伺服（`app.js` → `application/javascript`、`app.css` → `text/css`、其余 404——豁免 ≠ 任意文件可读，无文件系统访问、无路径穿越）。
- `build_app` 增挂 static 路由；壳路径与 `/api/time` 一并签名豁免（控制台须在输入 token 前可加载）。
- 删除 `src/assets/`（13 文件 vanilla 内联前端）、`tests/{dom_shim,frontend_nodes,frontend_smoke}.mjs`、`tests/web_frontend_runtime.rs`——前端逻辑改由 vitest 在 `spa/` 内直接测（`sign.js` HMAC 签名器、`reduce.js` 事件归约器），Rust 侧不再维护 DOM shim。
- `scripts/check-spa-drift.sh`：核对 `spa/dist` 入库产物与本地构建一致（内嵌是编译期快照，必须显式防「改了源码忘了重建 dist」的漂移）。
- `tests/auth.rs::shell_paths_are_exempt_but_api_is_not` 契约反转：`/static/app.js` 200 + 白名单外静态名 404 + API 未签名仍 401。
- `Cargo.toml` 移除失配的 tower-http（伺服已白名单内嵌，不再读盘）。

### 浏览器验收收口（playwright-core + 系统 chromium 实跑，8 步驱动脚本）

真实浏览器验收不是「无法自动化」而是缺基础设施：`/tmp` 下以 playwright-core 驱动系统 chromium（`--no-sandbox`）完成 interrupt / SSE 断连重连 / compact 回放全清单，两轮连续 8/8 PASS。验收抓出 5 个真实缺陷并当场修复：

1. **SPA 从未挂载**（任何浏览器里都是空壳）：`spa/src/main.jsx` 缺 `createRoot(...).render(...)`，旧产物 tree-shake 后整包为空。`html.rs` 加守护测试 `bundle_bootstraps_the_react_root`。
2. **中断无终帧，控制台永久 busy**：runner 的中断出口只发 `Status("interrupted")`。四处出口（循环头、tool 执行后、**LLM 流式中**、`apply_steer_batch` 硬取消）统一补发终帧 `Done`。`interrupt_emits_done.rs` 新增 3 测试钉契约；`hard_cancel_midstream.rs` 的 D2 契约按新语义修订——`Done` 是关闭 SSE 的**终帧**（不再是「正常完成」标记），`Status` 承载原因；D2 真正要防的「空 assistant 消息落库」断言原样保留。
3. **重连 cursor 跳头吞掉终帧**：断连期间 run 结束时，`after=<persisted head>` 跳过 `done` 帧 → UI 等 90s 超时。改为从 `lastSeq` 重放，且重放窗口封顶 400 帧（`REPLAY_CAP_FRAMES`，run 结束时终帧恒为 head，有界尾部重放仍收敛；无封顶时 4 万+增量帧回放会把标签页 O(n²) 渲染拖死）。`GET /api/sessions/:id` 暴露 `draining` 布尔（运行态可观测）+ `bugfix_contracts.rs` 契约测试。
4. **store 快照回放渲染空 transcript**：wire 上 ContentBlock 是 serde `tag = "kind"`，SPA `turnsFromMessages` 只匹配 `type` → 所有 block 静默失配，openDialog/reloadAfterDone/resume 全部渲染空。兼容两种 tag 并以 `kind` fixture 钉住 wire 契约；快照路径补 `usageFromMessages`（按消息行 usage 求和还原 ▲▼Σ 脚注，reload 后不再消失）。
5. **登录门控漏面板**：登录前 `NodesPanel` 已挂载发空 token 请求，控制台常驻「HMAC key data must not be empty」红字。Content 按 token 条件渲染。

### 测试面迁移（根级 tests/）

- `tests/running_mode_switch_e2e.rs`：弃用已删 `server` 动词 → `daemon --server`；裸 HTTP helper 全部 HMAC 签名化（fresh ts 防重放冲突），运行中拒绝切换语义复绿。
- 新增 `tests/daemon_smoke.rs`（308 行）进程级 e2e，单测走完整部署链路：
  1. 起真 server（端口 0，避免固定端口竞争）；
  2. 签名矩阵：裸 `/` 200、`/api/time` 免签 200、未签名 API 401、正确签名 200、过期 ts 401、body 篡改 401、同签重放 409；
  3. `daemon --client --name smoke-node-1` 注册：`addr` 记录来源 IP（schema v13 `nodes.last_addr`）、`last_seen_at` 随心跳循环推进、client 进程存活断言。

### SPA 控制台（crates/web/spa）

- React18 + antd + Vite；登录（token）→ 舰队节点面 → 对话面（SSE 流式 + interrupt）；`sign.js` 在浏览器侧实现与 `core/src/auth_sig.rs` 相同的 canonical 四行串 + HMAC-SHA256 签名，wire 格式由 daemon_smoke 的独立 python3 签名器与实机冒烟双重验证。
- 构建产物以固定文件名（无 content hash）入库：`index.html` + `static/{app.js,app.css}`，与 `html.rs` 的白名单一一对应。

### 回归门动作

- 清理 unused import；`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo build --release --workspace` 双绿。
- 一个已知红点的前因：并发流把 F3 失败路径语义改为 zero-resubmit（`crates/session/src/runner/mod.rs` 错误路径不再 re-absorb），`steer_batch_recovery.rs` 旧断言（consumed==2）随之失效，已在同树内对齐为新契约（consumed==1，无错误路径重投）并通过。该语义变更的完整记录见 `ctrlt-decouple-and-zero-resubmit.md`，本文件不重复。

### 文件面增删

- 新增：`crates/cli/src/daemon.rs`、`crates/cli/tests/cli_parse_daemon.rs`、`crates/core/src/auth_sig.rs`、`crates/web/src/auth_sig_mw.rs`、`crates/web/src/{api_control,control_state}.rs`、`crates/node/src/control.rs`、`crates/node/tests/runner_control.rs`、`crates/tui/src/mode_switch.rs`、`crates/web/spa/`、`tests/daemon_smoke.rs`、`crates/web/tests/{node_messages_relay.rs,support/}`、`scripts/{build-spa.sh,check-spa-drift.sh}`。
- 删除：`crates/client/`（整个 crate）、`crates/cli/src/{client,client_ops,client_stream}.rs`、`crates/cli/tests/client_session_parity_parse.rs`、`crates/web/src/assets/`、`crates/web/tests/{dom_shim,frontend_nodes,frontend_smoke}.mjs`、`crates/web/tests/web_frontend_runtime.rs`、`tests/client_server_smoke.rs`；`server`/`client`/`node` 三子命令与 `crates/cli/tests/cli_parse.rs` 中对应解析分支一并移除，`src/main.rs` 与 `src/lib.rs` 收敛到 daemon 分发。

## 真实测试输出

```
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [optimized] target(s) in 20.00s   # 0 warning

$ cargo test --workspace
passed=3394 failed=0   (223 个测试二进制全绿)

关键单测：
test daemon_server_and_client_end_to_end ... ok          (tests/daemon_smoke.rs, 12.03s)
test smoke_script_two_process_nodes_flow_passes ... ok   (tests/nodes_smoke_proc.rs, 18.63s)
test real_server_rejects_running_mode_switches_until_idle ... ok (tests/running_mode_switch_e2e.rs, 1.09s)

$ cargo build --release --workspace
    Finished `release` profile [optimized] target(s) in 1m 14s
    target/release/opencoder  19,375,760 bytes（单二进制，无 node 依赖）

$ cd crates/web/spa && npm test
 Test Files  2 passed (2)  /  Tests  20 passed (20)
$ bash scripts/check-spa-drift.sh
spa dist: no drift
```

release 二进制实机冒烟（127.0.0.1 临时端口，python3 独立签名器验证 wire 格式）：

```
-- unsigned GET /                    → 200 text/html; charset=utf-8（「Opencoder 舰队控制台」SPA 壳）
-- unsigned GET /api/time            → 200 {"server_time_ms":1787848015864}
-- unsigned GET /api/health          → 401
-- signed   GET /api/health          → 200 {"commit":"e061fc2","ok":true,...}
-- unsigned GET /static/app.js       → 200 application/javascript 470850b
-- unsigned GET /static/app.css      → 200 text/css 303b
-- daemon --client 注册 live-node-1 → signed GET /api/nodes → 200,
   {"addr":"127.0.0.1","first_seen":1787848015855,"last_seen_at":1787848025874,...}
   （addr 即 schema v13 nodes.last_addr 来源 IP；last_seen 晚 first_seen 10s = 心跳循环在跑）
-- 同签重放：first=200, second=409
```

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| daemon 模式校验/角色互斥/usage 提示 | `both_roles_err_even_though_clap_already_rejects_the_pair` 等 5 个 | `crates/cli/tests/cli_parse_daemon.rs` |
| client token 解析（flag>env、空 env 视同缺失、永不自动生成） | `client_token_flag_wins_over_env` / `client_token_blank_env_is_treated_as_missing` / `client_token_never_auto_generates` | `crates/cli/src/daemon.rs` |
| 节点名默认值唯一性 | `default_node_name_is_nonempty_dns_label_with_unique_suffix` | `crates/cli/src/daemon.rs` |
| 签名协议（canonical/窗口边界/常时比较/body hash） | 8 个 `auth_sig` 单测 | `crates/core/src/auth_sig.rs` |
| 中间件（未签名 401/错签 401/过期 401/重放 409/超限 413/body 篡改 401/`/api/time`+壳路径豁免/恶意 ts 401/签名 POST body 校验） | `api_without_signature_is_401` `replayed_signature_is_409` `oversized_body_is_413` `shell_paths_are_exempt_but_api_is_not` `time_endpoint_is_unsigned` `stale_timestamp_is_401` `malformed_timestamp_is_401` `signed_post_body_is_verified` 等 10 个 | `crates/web/tests/auth.rs` |
| SPA 内嵌（白名单伺服/壳引用可解析/禁外链/控制台入口） | `static_whitelist_serves_fixed_build_outputs` `shell_references_resolve_through_the_whitelist` `shell_has_no_external_references` `shell_is_the_console_entry` | `crates/web/src/html.rs` |
| SPA 前端逻辑（签名器/归约器/快照 usage/`kind` wire tag） | vitest 22 passed | `crates/web/spa/src/{sign.test.js,reduce.test.js}` |
| SPA 挂载守护（main.jsx 缺 render 曾致任意浏览器空壳） | `bundle_bootstraps_the_react_root` | `crates/web/src/html.rs` |
| 中断终帧（循环头/tool 后/LLM 流式中/steer 批硬取消） | `loop_head_interrupt_emits_done` `tool_exec_interrupt_emits_done` `llm_round_interrupt_emits_done` + `hard_cancel_midstream_no_empty_assistant`（修订契约） | `crates/session/tests/{interrupt_emits_done.rs,hard_cancel_midstream.rs}` |
| 会话快照 draining 可观测 | `get_messages_exposes_draining_flag` | `crates/web/tests/bugfix_contracts.rs` |
| 进程级 daemon e2e（server+client 注册/心跳/来源 IP/签名矩阵） | `daemon_server_and_client_end_to_end` | `tests/daemon_smoke.rs` |
| 运行中模式切换拒绝（真二进制，签名化后复绿） | `real_server_rejects_running_mode_switches_until_idle` | `tests/running_mode_switch_e2e.rs` |
| zero-resubmit 对齐 | `runner_consumes_batch_steers_with_failing_store` | `crates/session/tests/steer_batch_recovery.rs` |

## 边界

- 浏览器清单已自动化执行（playwright-core + 系统 chromium 151 `--no-sandbox`，8 步驱动：登录/控制台渲染/本地流式/断连徽标/SSE 重连/中断/compact/压缩后回放，连续两轮 8/8 PASS，证据截图 `01-fleet-console` 至 `07-compact-replay`）。未覆盖：多节点 fleet 下的重连与回放（本环境单 server）、小程序/窄屏布局、`favicon.ico` 404（外观噪音，壳无此引用）。
- 重放缓存为进程内存（重启清零、多实例不共享）——v1 既定取舍，见 `auth_sig_mw.rs` 模块注释。
- 固定文件名入库 = 无 content hash、无 cache-busting：SPA 迭代后必须重建 dist 并入库（`check-spa-drift.sh` 把守），浏览器端靠手动强刷；正式发布若需缓存策略再引入 hash 版本化。
- 行数门核对范围：本轮全部新增/迭代文件（新增最大 `crates/web/tests/node_messages_relay.rs` 358 行，迭代最大 `crates/tui/src/app.rs` 796 行）。
