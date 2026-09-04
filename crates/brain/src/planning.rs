//! Dynamic planner orchestration: framework prompt + vector-retrieved
//! capability manifest → LLM → validated [`DecisionTree`] → persisted
//! [`BrainPlanRecord`]; then dispatch by embedding the live situation and
//! walking the tree. I/O lives here, structure lives in [`crate::plan`].

use std::collections::HashSet;

use anyhow::{Context, Result};
use serde_json::json;

use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{BrainPlanRecord, BrainVectorHit};

use crate::error::{PlanGenerationFailed, PlanNotFound};
use crate::plan::{
    attach_topic_vectors, collect_topics, dispatch, validate, DecisionTree, DispatchOutcome,
};
use crate::runtime::{Runtime, PLAN_ID_PREFIX};

/// The framework prompt every planning call runs under — the single source
/// of truth for what the planner model is asked to produce. Kept `pub` so
/// callers (tests, audits, the web surface) can pin the exact contract.
pub const PLANNER_FRAMEWORK_PROMPT: &str = "\
你是「能力动态规划器」：把候选能力库组织成一棵决策树，让任何新情况都能被路由到最合适的一个能力。\n\
\n\
输入：\n\
1. 当前情况（situation）\n\
2. 候选能力清单：每项含 capability_id、类型、摘要、输入/输出描述，以及与当前情况的向量距离 distance（越小越相关；相似度 = 1 - distance）\n\
\n\
任务：输出一棵严格 JSON 的二叉决策树，把情况空间划分为若干区域，每个区域落到一个能力。\n\
\n\
构造规则：\n\
1. 只能引用候选清单中出现过的 capability_id，禁止编造。\n\
2. 分支节点（kind=branch）：topic 是不超过 16 字的判别主题短语，各分支 topic 必须彼此正交（一个情况只应强烈匹配一条路径）；yes 子树 = topic 命中的去向，no 子树 = 未命中的去向。\n\
3. 叶节点（kind=leaf）：capability_id + 一句 reason 说明为何这种情况归它。\n\
4. 深度不超过 4、叶子不超过 8；向量距离更近的能力放在更浅层（更可能被选中）。\n\
5. threshold 是路由阈值：某分支的余弦相似度 ≥ threshold 走 yes。常规语义嵌入取 0.35 左右；候选彼此语义接近时调低，松散时调高。\n\
6. 若所有能力都不适配，仍必须选最相关的一个作叶——树必须永远完整可达。\n\
\n\
输出（只输出 JSON，不要任何其它文字）：\n\
{\"threshold\":0.35,\"root\":{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"…\",\"reason\":\"…\",\"yes\":{…},\"no\":{…}}}\n\
叶节点形如：{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"…\",\"reason\":\"…\"}";

/// What one `dispatch_or_plan` call resolved to: the routed outcome plus the
/// plan that produced it (and whether that plan was minted by this call or
/// reused from the digest cache).
#[derive(Debug, Clone)]
pub struct Dispatched {
    pub record: BrainPlanRecord,
    pub outcome: DispatchOutcome,
    /// `true` when this call planned a fresh tree; `false` when it reused
    /// the cached newest plan for the situation digest.
    pub planned_fresh: bool,
}

impl Runtime {
    /// Plan a decision tree for one situation: embed → vector-search the
    /// capability library → prompt the planner model under
    /// [`PLANNER_FRAMEWORK_PROMPT`] → parse + validate against the retrieved
    /// candidate set → embed every branch topic → persist. LLM-side failures
    /// (stream error, unparseable reply, contract violation) surface as the
    /// typed [`PlanGenerationFailed`] marker; embed failures as
    /// [`crate::error::EmbeddingFailed`].
    pub async fn plan_decision_tree(
        &self,
        chat_model: &str,
        situation: &str,
        top_k: u32,
        now_ms: i64,
    ) -> Result<(BrainPlanRecord, DecisionTree)> {
        let hits = self.search(situation, top_k).await?;
        if hits.is_empty() {
            return Err(gen_failed(
                "capability library has no vector hits — create capabilities before planning",
            ));
        }
        let req = ChatRequest {
            model: chat_model.to_string(),
            messages: vec![
                json!({ "role": "system", "content": PLANNER_FRAMEWORK_PROMPT }),
                json!({ "role": "user", "content": build_user_prompt(situation, &hits) }),
            ],
            tools: Vec::new(),
            tool_choice: None,
            temperature: Some(0.2),
            max_tokens: Some(2048),
            reasoning_effort: None,
            cache_salt: None,
        };
        let raw = drain_chat(self.client.as_ref(), req)
            .await
            .map_err(|e| gen_failed(format!("planner chat failed: {e:#}")))?;
        let mut tree = parse_tree(&raw)
            .map_err(|e| gen_failed(format!("planner reply unparseable: {e:#}")))?;
        let ids: HashSet<&str> = hits.iter().map(|h| h.capability.id.as_str()).collect();
        validate(&tree, &ids).map_err(|e| gen_failed(format!("planner tree rejected: {e}")))?;
        let mut topics = Vec::new();
        collect_topics(&tree.root, &mut topics);
        let vecs = self.embed_many(&topics).map_err(|e| {
            anyhow::Error::new(crate::error::EmbeddingFailed {
                detail: format!("{e:#}"),
            })
        })?;
        attach_topic_vectors(&mut tree.root, &vecs)
            .map_err(|e| gen_failed(format!("topic vector attach failed: {e}")))?;
        let record = BrainPlanRecord {
            id: format!("{PLAN_ID_PREFIX}-{}", ulid::Ulid::new()),
            situation: situation.trim().to_string(),
            situation_digest: situation_digest(situation),
            chat_model: chat_model.to_string(),
            tree_json: serde_json::to_string(&tree).context("serialize decision tree")?,
            created_at: now_ms,
        };
        self.store.save_brain_plan(&record).await?;
        Ok((record, tree))
    }

    /// Route one situation through a persisted plan (looked up by id).
    /// Unknown id → typed [`PlanNotFound`]; embed outage →
    /// [`crate::error::EmbeddingFailed`]; a corrupt stored tree or an
    /// embedding-model mismatch is a 500-class plain anyhow error.
    pub async fn dispatch_decision_tree(
        &self,
        plan_id: &str,
        situation: &str,
    ) -> Result<(BrainPlanRecord, DispatchOutcome)> {
        let record = self.store.get_brain_plan(plan_id).await?.ok_or_else(|| {
            anyhow::Error::new(PlanNotFound {
                id: plan_id.to_string(),
            })
        })?;
        self.dispatch_record(&record, situation).await
    }

    /// The one-call dynamic scheduler: reuse the newest cached plan for this
    /// situation digest unless `replan` — plan first when there is nothing
    /// to reuse — then route the situation through it.
    pub async fn dispatch_or_plan(
        &self,
        chat_model: &str,
        situation: &str,
        top_k: u32,
        replan: bool,
        now_ms: i64,
    ) -> Result<Dispatched> {
        let digest = situation_digest(situation);
        let cached = if replan {
            None
        } else {
            self.store.latest_brain_plan_for(&digest).await?
        };
        match cached {
            Some(record) => {
                let outcome = self.dispatch_record(&record, situation).await?.1;
                Ok(Dispatched {
                    record,
                    outcome,
                    planned_fresh: false,
                })
            }
            None => {
                let (record, _) = self
                    .plan_decision_tree(chat_model, situation, top_k, now_ms)
                    .await?;
                let outcome = self.dispatch_record(&record, situation).await?.1;
                Ok(Dispatched {
                    record,
                    outcome,
                    planned_fresh: true,
                })
            }
        }
    }

    /// Shared tail of both dispatch entries: parse the stored tree, embed
    /// the situation, walk. Kept private so every public path funnels
    /// through the same error semantics.
    async fn dispatch_record(
        &self,
        record: &BrainPlanRecord,
        situation: &str,
    ) -> Result<(BrainPlanRecord, DispatchOutcome)> {
        let tree: DecisionTree = serde_json::from_str(&record.tree_json)
            .with_context(|| format!("stored plan {} tree is corrupt", record.id))?;
        let emb = self.embed_one(situation)?;
        let outcome = dispatch(&tree, &emb)
            .with_context(|| format!("dispatch through plan {} failed", record.id))?;
        Ok((record.clone(), outcome))
    }
}

/// Render the planner's user message: the situation plus the candidate
/// manifest, one line per capability with its vector distance.
fn build_user_prompt(situation: &str, hits: &[BrainVectorHit]) -> String {
    let mut lines = vec![
        format!("当前情况：{}", situation.trim()),
        String::new(),
        "候选能力清单：".to_string(),
    ];
    for h in hits {
        let c = &h.capability;
        lines.push(format!(
            "- {} | 类型:{} | 摘要:{} | 输入:{} | 输出:{} | distance={:.3}",
            c.id, c.capability_type, c.summary, c.input_desc, c.output_desc, h.distance
        ));
    }
    lines.join("\n")
}

/// Parse the planner reply into a [`DecisionTree`]. Tolerates the usual
/// wrapper noise (prose, ```json fences) by slicing the first `{` to the
/// last `}`; anything else fails as a plain anyhow error the caller maps to
/// the typed generation marker.
fn parse_tree(raw: &str) -> Result<DecisionTree> {
    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```");
    let cleaned = cleaned.trim_end_matches("```").trim();
    let Some(start) = cleaned.find('{') else {
        anyhow::bail!("planner reply contains no JSON object");
    };
    let Some(end) = cleaned.rfind('}') else {
        anyhow::bail!("planner reply JSON object is unterminated");
    };
    serde_json::from_str(&cleaned[start..=end])
        .context("planner reply is not a valid decision tree")
}

/// Drain one scripted chat stream to its final text — the same discipline
/// `generate_title` uses: accumulate `TextDelta`s, let `Completed.text`
/// override, clear on `Retrying` (a retry restarts the reply from scratch).
async fn drain_chat(client: &dyn ChatStream, req: ChatRequest) -> Result<String> {
    let mut rx = client.chat_stream(req)?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(d) => text.push_str(&d),
            LlmEvent::Retrying { .. } => text.clear(),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Error(e) => anyhow::bail!("{e}"),
            LlmEvent::ReasoningDelta(_)
            | LlmEvent::ToolCallStart { .. }
            | LlmEvent::ToolCallDelta { .. } => {}
        }
    }
    if text.trim().is_empty() {
        anyhow::bail!("planner produced an empty reply");
    }
    Ok(text)
}

/// Stable 128-bit digest (two seeded FNV-1a passes) of a situation text —
/// the dispatch cache key. Not a security primitive: it only needs to be
/// deterministic and collision-free across realistic situations.
pub fn situation_digest(situation: &str) -> String {
    fn fnv64(seed: u64, bytes: &[u8]) -> u64 {
        bytes.iter().fold(seed, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }
    let bytes = situation.trim().as_bytes();
    let a = fnv64(0xcbf2_9ce4_8422_2325, bytes);
    let b = fnv64(0x9e37_79b9_7f4a_7c15, bytes);
    format!("{a:016x}{b:016x}")
}

fn gen_failed(detail: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(PlanGenerationFailed {
        detail: detail.into(),
    })
}
