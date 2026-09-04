Commit: 8ca4d0e（统一收口落库，348 文件含 brain 全量；尾笔 66a3247 补最后一笔 changelog）

# opencoder-brain — 项目目标/能力库

能力库 = 「这个项目有什么能力/目标、怎么验收」的语义记忆：录入能力条目（类型/一句话描述/输入/输出/工程输入多行），嵌入为向量，按自然语言 query 语义检索。供人与 agent 查「要做什么、怎么算做好」。

## 结构（纯函数式，无 class）

- `types.rs`：`CapabilityInput`（wire 数据，Deserialize）。
- `domain.rs`：纯函数 `validate`（字段非空/长度上限/工程输入 ≤64 条）、`compose_embed_text`（五段稳定拼装成单条嵌入文本）、`f32_slice_to_le_bytes` / `le_bytes_to_f32_slice`（LE f32 编解码，store 向量 BLOB 的编码约定）。
- `runtime.rs`：`Runtime { store, client, model }` 数据 struct；`upsert_capability`（validate→compose→embed→`brain-{ULID}`→**单事务组合写**：向量字节先算好装入 `BrainVectorWrite` 传入 `create_brain_capability_with_vector`，capability+eng_inputs+vector 同提交/回滚）、`update_capability`（重嵌入、保留 created_at，同款 `update_brain_capability_with_vector`——无「新内容配旧向量」残留窗口）、`delete/get/list`、`search(query, k)`（embed query→`vector_distance_cos` 排序）。
- `plan.rs`（纯域）：`DecisionTree{threshold,root}` / `PlanNode`（serde tag=kind，branch 携 topic+topic_vec|leaf 携 capability_id）；`validate`（叶 id ⊆ 候选集、id 唯一、深度≤6/叶≤16、topic≤64 字、threshold∈(0,1]）、`collect_topics` 先序 + `attach_topic_vectors`（批量挂载、L2 归一、基数/维度校验）、`dispatch`（余弦 ≥threshold 走 yes，返回 capability_id+全路径 `DispatchStep` 审计）。
- `planning.rs`（编排）：`PLANNER_FRAMEWORK_PROMPT` 六条构造规则的框架提示词（pub 常量）；`plan_decision_tree`（embed→向量检索 top_k→LLM 建树→剥围栏解析→候选集校验→批量 embed 主题→持久化 `brain-plan-{ULID}`）、`dispatch_decision_tree(plan_id, situation)`、`dispatch_or_plan`（按 `situation_digest`（双种子 FNV-1a 128bit）缓存复用，replan 强制重建）；`chat_model` 独立字段（builder `with_chat_model`，生产=small_model）。
- `error.rs`：typed marker（手写 Display/Error，零依赖）——`EmbeddingFailed { detail }`：embed 上游失败（HTTP 错/基数不符/空向量）统一装进 `anyhow::Error`，web 层 `downcast_ref` 判 502；`BrainNotFound { id }`：update 未命中 id → web 层判 404（消除最后一处 `contains("not found")` 字符串分界；insert/update 之后的 not-found 不变量仍为 anyhow context → 500）。

## 依赖方向与接缝

- 向量存 store schema v15 三张表（`brain_capabilities` / `brain_eng_inputs` / `brain_vectors`，详见 [agents/store](../store/index.md)）；`search_brain_vectors` 内置 model 过滤（规避跨模型 dim 不匹配）。
- embed 走 `ChatStream::embed`（OpenAI 兼容 `/embeddings`，默认 bail，Mock 为 8 维确定性单位向量）——见 [agents/llm](../llm/index.md)；模型由 `Config::embedding_model`（默认 `text-embedding-3-small`）配置，复用主 endpoint。
- 决策树计划存 store schema v18 `brain_plans`（+ `idx_brain_plans_digest`，`latest_brain_plan_for` 按 created_at DESC,rowid DESC 取最新）；tree_json 内嵌 topic 向量，读取零额外查询。
- web 层 `api_brain.rs` 九条路由挂 `/api` 前缀（自动受 HMAC 签名保护），SPA「项目目标」tab；embed 失败→502、`PlanNotFound`→404、`PlanGenerationFailed`→502。见 [agents/web](../web/index.md)。

## 测试

`crates/brain/tests/runtime.rs` 8 例（Mock+内存库零 token 闭环：录入→自检索 distance≈0、update 重嵌入、级联删、校验拒写、k 截断、not found、embed 失败 typed 且零残留、update embed 失败保留旧行——原子性）；store 层 `tests/brain_store.rs` 9 例（CRUD/级联/vector_distance_cos 实际排序/model 过滤/v14→v15 migration/组合写 create 三件套/组合写 update 替换语义）；web 层 `tests/web_brain.rs` 6 例（CRUD/search/400/404/502 降级/未签名 401）；`tests/planning.rs` 10 例 + `tests/web_brain_plans.rs` 4 例（决策树：同文本 cosine=1.0≥0.98 走 yes / 异文本≤0.95 走 no——阈值确定性来自 Mock FNV 向量；缓存复用零 LLM 调用）；SPA `brainPanel.dom.test.jsx`。
