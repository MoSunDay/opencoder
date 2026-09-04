Commit: 1ac4afa（feat 落库；docs 回填本笔）

# opencoder-brain：动态规划调度——框架提示词 + 向量表 → 决策树 → 按情况动态路由

## Context

能力库（`opencoder-brain`，schema v15）已支持录入与语义检索，但「来了一个新情况该用哪个能力」仍靠人读检索结果。本轮补上调度层：一条**框架性提示词**（`PLANNER_FRAMEWORK_PROMPT`，公开常量）约束 LLM 把「向量检索出的候选能力清单」组织成一棵**二叉决策树**（分支=判别主题、叶=能力），树随分支主题向量一起持久化；此后任意新情况只需**一次 embedding** 沿树路由（分支处 `cosine(情况向量, 主题向量) ≥ threshold` 走 yes），即得唯一能力 + 全路径审计轨迹。规划结果按「情况文本稳定摘要」缓存，重复情况零 LLM 成本复用。

## Change Summary

- **brain `plan.rs`（纯域，300 行）**：`DecisionTree { threshold, root }` / `PlanNode`（serde tag=`kind` 的 branch|leaf，branch 携 `topic` + `topic_vec`）；`validate`（id 唯一非空、topic ≤64 字符、叶 capability_id ⊆ 候选集、深度 ≤6/叶 ≤16 硬顶、threshold ∈(0,1]）；`collect_topics` 先序收集 + `attach_topic_vectors` 批量挂载（L2 归一、基数/维度校验）；`dispatch` 全量余弦走树 → `DispatchOutcome { capability_id, reason, path }`（`DispatchStep` 记 node/kind/score/taken 审计轨迹）。纯函数零 I/O，全部可无 LLM 单测。
- **brain `planning.rs`（编排，277 行）**：`PLANNER_FRAMEWORK_PROMPT`（六条构造规则：只准引用候选 id / topic 正交 ≤16 字 / 深度 ≤4 叶 ≤8 / 距离近者浅层 / threshold 语义与取值指引 / 树必须完整可达）+ 用户侧清单渲染（id/类型/摘要/输入输出/distance）；`Runtime::plan_decision_tree`（embed → 向量检索 top_k → 框架提示词 LLM 调用（temperature 0.2, max_tokens 2048，drain 模式同 title 生成）→ 剥代码围栏解析 → 候选集校验 → 批量 embed 主题挂载 → 持久化）；`Runtime::dispatch_decision_tree`（按 id 取树 + embed 情况 + 走树）；`Runtime::dispatch_or_plan`（一次调用动态调度：摘要缓存命中即复用、`replan` 强制重规划）；`situation_digest`（双种子 FNV-1a 128bit，仅缓存键非安全用途）。
- **错误标记（typed）**：`PlanNotFound { id }`（dispatch/get 未知计划 → web 404）、`PlanGenerationFailed { detail }`（流失败/回复不可解析/契约违规/空库 → web 502）；embed 故障沿用 `EmbeddingFailed` → 502；存储树损坏/维度失配保持 plain anyhow → 500。`parse_tree` 失败同样收敛进 `PlanGenerationFailed`（修复过程中发现的真缺口：首版 `?` 直传会漏成 500）。
- **store（schema v17→v18）**：`brain_plans`（id/situation/situation_digest/chat_model/tree_json/created_at）+ `idx_brain_plans_digest(digest, created_at)`；`Store` trait 增 `save_brain_plan`/`get_brain_plan`/`latest_brain_plan_for`（默认 bail，libsql 委托 brain.rs 新自由函数；latest 按 `created_at DESC, rowid DESC` 全序稳定，同毫秒以 rowid 决胜——与 node_tasks FIFO 同约定）。
- **web `/api/brain/*`**：`POST /plans`（201 返回 record+解析树；situation 空 → 400；top_k 同 search 的 clamp 策略；model 可选覆盖，默认链 runtime `chat_model`）；`GET /plans/:id`（404/损坏 500）；`POST /dispatch`（带 `plan_id` 走精确计划；缺省走动态调度：缓存命中复用否则先规划，`replan` 强制；返回 capability_id/reason/path/planned_fresh）。`map_plan_error` 三标记分派 404/502/500。生产装配 `Runtime::with_chat_model(cfg.small_model_or_primary())`——规划用小模型（结构化输出、低成本），embed 模型不变。
- **runtime 小改**：字段 `pub(crate)` 化 + `chat_model` 字段（默认=embed 模型 id，builder 覆盖）+ `embed_many` 批量版（`embed_one` 收敛其上，同为 `EmbeddingFailed` 语义）+ `PLAN_ID_PREFIX`。

## 测试（规则 01/02：全绿清单）

- `crates/brain/tests/planning.rs`（10 例）：规划→持久化→按主题路由（同文本 cosine=1.0 ≥0.98 走 yes / 异文本 ≤0.952 走 no，阈值确定性来自 MockChatClient FNV 哈希向量）；未知计划 → typed `PlanNotFound`；伪造 capability_id / 不可解析回复 → typed `PlanGenerationFailed`（围栏包裹回复同时验证解析鲁棒性）；空库规划失败且零 LLM 调用（`call_count()==0`）；`dispatch_or_plan` 缓存命中（第二次无脚本零 LLM 调用）+ replan 换树；损坏树 → 非 404/502 的 500 类；纯域 validate/attach/dispatch/digest 4 例（重复 id、越界 threshold、基数/维度、缺 topic_vec、维度失配报错不静默路由）。
- `crates/web/tests/web_brain_plans.rs`（4 例）：plan 201 → GET :id 200/404 → dispatch 按 plan_id 双向路由；无 plan_id 先规划后缓存复用（planned_fresh true→false）；错误契约 400/404/502；未签名请求经 HMAC 网关 401。
- `crates/store/tests/brain_store.rs`（+3 例，12 全绿）：save/get/latest-by-digest 往返（新者胜）；同毫秒 rowid 决胜；v17→v18 手工旧库迁移（表+索引落位、版本=18、迁移后 API 往返）+ 既有 v14→v15 断言同步 18。

## 回归证据

- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0。
- `cargo test --workspace --no-fail-fast` → exit=0：302 suite ok / 0 failed / 4404 passed（含本轮新增 3 suite 17 例；store 迁移 suites 的 version 断言随 v18 同步）。

## 设计取舍备忘

- **阈值判定而非 LLM 在线判定**：路由是纯余弦比较，同树同向量结果确定、可离线复算审计；LLM 只在「建树」时参与一次。
- **树缓存按精确文本摘要**：同文复用零成本；近义不同文走新规划（语义相似性本就由树内 threshold 表达，不需要模糊缓存）。
- **`topic_vec` 内嵌树 JSON**：读取侧零额外查询；写入侧一次批量 embed。
- **规划模型与嵌入模型解耦**：`chat_model` 独立字段 + 请求级覆盖，生产默认小模型。
