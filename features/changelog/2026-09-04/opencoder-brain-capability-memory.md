Commit: pending（共享工作树多会话并行迭代中，brain 功能未单独提交，随收口会话统一落盘）

# opencoder-brain：项目目标/能力库——录入、语义向量检索与 Web「项目目标」面板

## Context

项目缺一个「这个项目有什么能力/目标、怎么验收」的语义记忆：目标与能力散落在对话与文档里，人和 agent 都无法按自然语言查询。本轮新增 `opencoder-brain` crate + store schema v15 三张 brain 表 + web `/api/brain/*` + SPA「项目目标」tab，形成录入→嵌入→语义检索闭环。已确认 libsql 0.9.30 bundled SQLite 内置 `vector32`/`vector_distance_cos`（经验证 blob 与 JSON 两种绑定均可，选 blob 与存储编码统一），无需升级依赖。

## Change Summary

- **store（schema v14→v15）**：`brain_capabilities`（主表，ULID id）/ `brain_eng_inputs`（子表，ON DELETE CASCADE，position 定序）/ `brain_vectors`（capability_id PK，`emb BLOB`=LE f32，dim+model）。迁移与并行会话的 project 表合入同一 `if from < 15` 块（v14→v15 一次建齐）；post-migration 索引 `idx_brain_eng_inputs_cap`。`Store` trait 增 7 个默认-bail 方法（create/update/delete/get/list/upsert_vector/search_brain_vectors）；`search` 用 `vector_distance_cos(v.emb, vector32(?))` 排序并按 model 过滤（规避跨模型 dim 不匹配报错）。
- **llm embed**：`ChatStream::embed(&[String], model)` 默认 bail + `Arc` 转发；`ChatClient` 非流式 POST `{base}/embeddings`（`build_embed_body`/`parse_embeddings_response` 纯函数化，按 `data[i].index` 稳定归位）；`MockChatClient` 确定性 8 维单位向量（`MOCK_EMBED_DIM`，`embed_calls()` 可断言）→ 零 token 测试。core `Config.embedding_model: Option<String>` + `embedding_model_id()`（默认 `text-embedding-3-small`，复用主 endpoint）。
- **brain crate（纯函数式）**：`domain.rs` 纯函数（validate 字段/长度/条数校验、compose_embed_text 五段稳定拼装、f32↔LE bytes 编解码）；`Runtime { store, client, model }`——upsert（校验→嵌入→`brain-{ULID}` 落库+向量）、update（重嵌入、保留 created_at）、delete/get/list、search(query, k)。embed 失败包 `"embedding failed"` context，作为 web 层 502/400 分界。
- **web**：`api_brain.rs` 六条路由（GET/POST `/api/brain/capabilities`、GET/PUT/DELETE `/:id`、POST `/api/brain/search`，k 默认 10 上限 50）挂 `/api` 前缀自动受 HMAC 签名保护；`AppState.brain`（全仓库 43 处构造点补字段）；`serve()` 按 handle.rs 同款 Config→ChatClient 构造真实 client，失败降级 bail-only client（服务照常启动、brain 接口 502 清晰报错）。
- **SPA**：`brainPanel.jsx` 纯函数组件——语义搜索（展示 distance `.toFixed(4)`、编辑回填）、录入/编辑 Form（工程输入 Form.List 动态增删）、能力列表 Table + Popconfirm 删除；`main.jsx` 菜单加「项目目标」。dist 经 `build-spa.sh` 重建、`check-spa-drift.sh` 无漂移。

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| store CRUD/级联/向量幂等 | `crates/store/tests/brain_store.rs` 1–4（update replace 语义、delete 级联清 eng_inputs+vectors、upsert_vector 不重复行） |
| 向量检索正确性 | 同上 5–7：`vector_distance_cos` 实际排序（[1,0,0,0] query → e1 d≈0 / e2 d≈1 / e3 d≈2）、model 过滤、v14→v15 migration |
| embed 请求/解析/降级 | `crates/llm`：`build_embed_body`/`parse_embeddings_response` 单测 + `tests/embeddings.rs` 本地 fake `/embeddings` 服务器（URL 拼接/鉴权头/乱序 index 归位/上游错误）+ mock 确定性（同文本同向量、单位范数、embed_calls） |
| config | `embedding_model_id` 默认/覆盖两分支 |
| brain 领域 | `crates/brain/src/domain.rs` 10 例（validate 各分支、compose 稳定、编解码往返含 NaN 位级） |
| brain 运行时闭环 | `crates/brain/tests/runtime.rs` 6 例（录入→自检索 d≈0、update 重嵌入后旧文本不再 top1、删后检索空、校验失败不入库、k 截断+升序、not found） |
| web e2e | `crates/web/tests/web_brain.rs` 6 例（CRUD roundtrip、search top1+d≈0、400 校验、404/空 query、降级 client→502 且读路径 200、未签名→401） |
| SPA | `crates/web/spa/src/brainPanel.dom.test.jsx` 5 例（表单控件/Form.List 增行、列表渲染、搜索触发+distance 渲染、Popconfirm 删除、编辑回填→PUT） |

## 回归（规则 02）

- `cargo test -p opencoder-core -p opencoder-llm -p opencoder-store -p opencoder-brain -p opencoder-web`：全绿（含 3994 通过的 workspace 基数中本功能涉及的 5 个 crate）。
- `cargo test --workspace`：本功能相关 crate 全绿；4 个失败 suite（`daemon_smoke`/`nodes_smoke_proc`/`running_mode_switch_e2e`/cli 侧）由并行会话在途的 server/client 二进制迁移引起（`daemon --server` 现报「moved to the dedicated server binary」，属 dag/server/agent 批次未收口状态），与 brain 改动无关——归因方法：以 `opencode-server --help` 实测 + 失败点全部位于 daemon 启动/CLI 解析路径。
- `npm test`（spa）：298 用例全绿；`scripts/build-spa.sh` 重建 dist；`scripts/check-spa-drift.sh` 无漂移。

## 评审后加固（P1/P2，同日第二轮）

评审五问指出三项后续：P1 跨表非原子、P2a 字符串错误分界、P2b embed 零重试。本轮全部落地：

- **P1 单事务组合写**：store 新增 `BrainVectorWrite { dim, model, emb(LE bytes), embedded_at }` 与 `Store::create/update_brain_capability_with_vector`（默认 bail；libsql 实现为单 `run_tx`，向量 INSERT OR REPLACE SQL 收敛到 `insert_or_replace_vector` 单点）。brain runtime 的 upsert/update 改为「先 embed、再单事务落库」，删除两步 `write_vector`——消除「条目落库但向量缺失（永不被检索命中）」与「update 残留旧向量（新内容配旧嵌入）」两个跨表部分写窗口。
- **P2a typed error**：brain crate 新增 `src/error.rs::EmbeddingFailed`（手写 Display/`std::error::Error`，零新依赖）；`embed_one` 三类失败（client 调用错/基数不符/空向量）统一装进该类型（上游链折叠进 `detail`）；web `map_brain_error` 改 `downcast_ref::<EmbeddingFailed>()` 判 502，删除 `contains("embedding failed")` 字符串耦合（模块 doc 与降级注释同步改写；502 响应体仍含 `embedding failed:` 前缀，契约不变）。
- **P2b embed 重试+超时预算**：`post_embeddings` 复用 `retry.rs` 既有纯策略（send 错误/408/425/429/5xx → `AttemptOutcome`/`retry_decision`；`EMBED_MAX_ATTEMPTS=3` 次尝试、指数退避+jitter；解析/数量校验不重试），请求 builder 加每请求 60s 总超时 `EMBED_REQUEST_TIMEOUT`（远紧于流式 10min read_timeout）；终态错误附 attempts 数。

### 测试清单（规则 01/03）

| 层 | 用例 |
|---|---|
| store 集成 | `crates/store/tests/brain_store.rs` +2（共 9）：组合 create 三件套齐（get 含有序 eng_inputs + search d≈0 + 向量表恰 1 行）；组合 update 替换语义（emb-b 新 model 命中 d≈0 / emb-a 旧 model 检索空 / 向量表仍 1 行——替换而非残留复制） |
| llm 集成 | `crates/llm/tests/embeddings.rs` +3（共 9；基建扩展 `serve_many` 按序多响应+请求计数）：500→200 恰重试一次（计数=2）；400 fail-fast（计数=1，错误含 status+body）；500×3 预算耗尽（计数=3，错误含 "after 3 attempts"） |
| brain 集成 | `crates/brain/tests/runtime.rs` +2（共 8；本地 `BrokenEmbedClient` 三模式 Bail/Cardinality/EmptyVector）：upsert embed 失败 → `downcast_ref::<EmbeddingFailed>()` 命中且 store 零残留；update embed 失败 → 旧行内容/created_at/eng_inputs 原样保留（原子性回归锚点） |
| web e2e | `crates/web/tests/web_brain.rs` 6 例不变全绿（degraded 502 的 `"embedding failed"`+`"llm endpoint unavailable"` 断言在新 Display 下原样满足） |

### 回归（规则 02，第二轮）

- `cargo test -p opencoder-core -p opencoder-llm -p opencoder-store -p opencoder-brain`：全绿（llm 165 例、store 184 例、brain 18 例、core 全过）；`cargo test -p opencoder-web`：30 suite / 132 例全绿。
- `npm test`（spa）：298/298 全绿；`scripts/check-spa-drift.sh`：无漂移（本轮未改 SPA 源码，dist 不需重建）。
- `cargo test --workspace`：仍被并行在途批次阻塞——本轮实测两处**实时编译错误**：`opencoder-dag-runtime`（`root_path_value` 未定义）、`opencoder-session`（`opencode_core` 未解析，agent 批次重命名中途），（再次重试又命中 `opencoder-session` 测试目标：`OVERRIDE_LOCK`/`HOME_LOCK`/`opencode_llm` 未解析）——阻塞点随并行编辑进度移动，始终不在 brain 改动面（core/llm/store/brain/web 五 crate 定向全绿已闭合本功能自身的 gate）；workspace 级 gate 待该批次收口后闭合。
- Commit：维持 pending——共享文件（root `Cargo.toml`、`crates/store/src/{lib,store}.rs`、`libsql_store/mod.rs`、`crates/web/src/lib.rs`、`crates/core/src/config.rs` 等）的未提交 diff 同时裹挟并行批次的 dag/agent/server 迁移改动，hunk 级拆分在并行编辑活动下不可靠（有覆盖他人未提交工作的风险）；随收口会话统一落库后回填 hash。

## 评审后打磨（P3，同日第三轮）

- **P3a typed not-found**：`crates/brain/src/error.rs` 增 `BrainNotFound { id }` marker（手写 Display/Error，Display 保持历史 `brain capability not found: {id}` 响应体形状，404 断言零改动）；`runtime.rs::update_capability` 前置存在性检查由 `with_context` 字符串改为 typed marker——insert/update 之后的 not-found 不变量仍为 anyhow context（500 语义不变）；`api_brain.rs` update handler 的 `contains("not found")` → `downcast_ref::<BrainNotFound>()` → 404。brain 错误映射至此**零字符串分界**（502/404 双 typed marker）。
- **P3b step-wise 写方法 doc 护栏**：`Store::create_brain_capability` / `update_brain_capability` trait 文档注明 runtime 路径必须走 `*_with_vector` 单事务组合写（capability+eng_inputs+vector 原子提交），逐步式仅供 store 级工具/测试，防误用回退引入跨表部分写窗口。
- **P3c embeddings Retry-After**：`post_embeddings` 可重试状态分支采纳 `Retry-After` 头（drain body 前捕获；整数秒优先、HTTP-date 回退复用 `crate::http_date::parse_http_date_to_secs`，与 chat 路径同款解析），经纯函数 `retry_delay`（floor 1s / cap 120s / 与指数退避取大）**单次 sleep**——对齐 chat 路径；传输错误分支（无状态码无头）维持裸退避。
- **P3 未做**：brain 语义检索接入 session/agent 提示词——brief 明示后续期。

### 测试清单（规则 01/03）

| 层 | 用例 |
|---|---|
| brain 集成 | `crates/brain/tests/runtime.rs::update_unknown_id_fails` 扩展：`downcast_ref::<BrainNotFound>()` 命中且 `Display == "brain capability not found: brain-nope"`（套件仍 8 例） |
| llm 集成 | `crates/llm/tests/embeddings.rs` +1（共 10）：`embed_honors_retry_after_header_on_429`——429+`Retry-After: 0`（floor 至 1s）后成功，请求计数==2，elapsed ≥ 1s 下界断言（裸退避 ≤0.75s 可判别；不设上界防抖动）；hint cap 不重复测（`retry.rs` 已有单测） |
| web e2e | `web_brain.rs` PUT 未知 id 用例补 404 body 断言 `"brain capability not found"`（仍 6 例，响应体形状回归锚点） |
| store | 无行为变更（doc 注记），`tests/brain_store.rs` 9 例原样全绿 |

### 回归（规则 02，第三轮）

- 定向 gate（本轮独立复跑核证）：`cargo test -p opencoder-brain`（10 lib + 8 runtime 全绿）、`-p opencoder-llm`（166 例：111 lib + 10 embeddings + connect_retry/headers/lower_messages/mock_contract 等）、`-p opencoder-store --lib`（25）、`-p opencoder-web --test web_brain`（6）；`cargo clippy -p opencoder-{brain,llm,web,store} --all-targets` 0 warning。
- `cargo test --workspace --no-fail-fast`（本轮首次拿到**完整**计数——此前两轮被 fail-fast 截断误读）：**291/294 suite ok、4350 passed**；仅 3 个失败 suite，全为根 crate 进程级冒烟（`daemon_smoke` / `nodes_smoke_proc` / `running_mode_switch_e2e`，共 4 例），根因同源：server 迁移批次已把 `opencode daemon --server` 改为重定向专用 `opencode-server` 二进制（实测打印 "has moved to the dedicated server binary"），三文件仍 spawn 旧入口等待该批次配套更新（`crates/server` dev-deps 已备 tempfile 而测试未落、迁移 changelog 未写、03:02 仍在编辑 `crates/agents`+`web/tests`）——不在 brain 改动面，且当日已有两起并行同文件编辑竞态（子代理实测编辑丢失），不代改。
- 另：本轮 workspace 编译面曾被并行实时编辑两次打断（store `project_factory.rs` E0277 等），均其自行修复后恢复。
- Commit：维持 pending——统一提交条件（并行批次收口 + workspace 全绿）未满足；**收口判据已量化：上述 3 个冒烟 suite 转绿 + server 迁移 changelog 落盘后即可统一落库并回填 hash**。
