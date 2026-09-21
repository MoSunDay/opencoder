//! Startup seeding of the built-in review release-gate DAG definitions.
//!
//! WHY: a fresh install otherwise exposes an empty `dag_defs` table, so the
//! three review gates (full acceptance environment+harness, quick harness,
//! quick code review) are invisible until somebody hand-posts specs. Seeding
//! at boot makes the gates discoverable out of the box on every fresh node.
//!
//! Skip-don't-overwrite rule: seeding checks the existing names and SKIPS
//! any def already present — never an upsert. Operators edit these defs
//! through `POST /api/dag/defs` (which upserts by name), and a server
//! restart must not clobber those edits with the seed copy. Every failure
//! mode (list error, invalid built-in spec, insert error) warns and moves
//! on: a broken seed must never block boot.

use opencoder_dag::{DagSpec, StepKind, StepSpec};
use opencoder_store::fleet::FleetStore;
use std::{collections::HashSet, sync::Arc};

/// Seed the three review release-gate defs into the fleet definition store
/// (`fleet_definitions`, the store behind `GET/POST /api/dag/defs`).
/// Idempotent by name; errors are logged, never propagated: the server stays
/// useful without the seeds.
pub(crate) async fn seed_review_dags(fleet: &Arc<FleetStore>) {
    let specs = [
        spec_full_acceptance(),
        spec_harness_quick(),
        spec_code_quick(),
    ];
    let existing: HashSet<String> = match fleet.definitions("dag").await {
        Ok(defs) => defs
            .into_iter()
            .filter_map(|d| d["name"].as_str().map(str::to_string))
            .collect(),
        Err(e) => {
            tracing::warn!(?e, "seed dag defs skipped: fleet.definitions failed");
            return;
        }
    };
    let mut seeded = 0usize;
    let mut skipped = 0usize;
    for spec in specs {
        if existing.contains(&spec.name) {
            skipped += 1;
            continue;
        }
        if let Err(problems) = opencoder_dag::validate(&spec) {
            // A seed failing its own gate is a seeding bug: warn and skip
            // this def; never panic and never block boot.
            tracing::warn!(
                name = %spec.name,
                problems = problems.join("; "),
                "seed dag def skipped: built-in spec failed validation"
            );
            skipped += 1;
            continue;
        }
        let body =
            crate::api::catalog::dag_definition(&spec, None, opencoder_core::message::now_ms());
        match fleet.put_definition("dag", &spec.name, &body).await {
            Ok(()) => seeded += 1,
            Err(e) => {
                tracing::warn!(name = %spec.name, ?e, "seed dag def insert failed");
                skipped += 1;
            }
        }
    }
    tracing::info!(seeded, skipped, "review release-gate dag defs seeded");
}

fn wasm_step(name: &str, depends_on: &[&str], command: &str, timeout_secs: u64) -> StepSpec {
    StepSpec {
        name: name.to_string(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        kind: StepKind::Wasm {
            command: command.to_string(),
            sandbox: None,
        },
        timeout_secs: Some(timeout_secs),
    }
}

fn agent_step(name: &str, depends_on: &[&str], prompt: &str) -> StepSpec {
    StepSpec {
        name: name.to_string(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        kind: StepKind::Agent {
            prompt: prompt.to_string(),
            agent: None,
            model: None,
            how_append: None,
        },
        timeout_secs: Some(1800),
    }
}

/// Full acceptance chain: bring up the EOB environment, then bare metal,
/// then run the full harness against it.
fn spec_full_acceptance() -> DagSpec {
    DagSpec {
        max_concurrency: 4,
        name: "review-full-acceptance".to_string(),
        description: None,
        steps: vec![
            wasm_step("env-eob-up", &[], "env_eob_up.wasm", 900),
            wasm_step(
                "env-baremetal-up",
                &["env-eob-up"],
                "env_baremetal_up.wasm",
                900,
            ),
            wasm_step(
                "harness-full",
                &["env-baremetal-up"],
                "harness_runner.wasm full",
                3600,
            ),
        ],
    }
}

/// Quick harness loop: no environment bring-up, just the quick suite.
fn spec_harness_quick() -> DagSpec {
    DagSpec {
        max_concurrency: 4,
        name: "review-harness-quick".to_string(),
        description: None,
        steps: vec![wasm_step(
            "harness-quick",
            &[],
            "harness_runner.wasm quick",
            1800,
        )],
    }
}

/// Quick agent-only code review chain. The workspace path is baked into the
/// prompts because `DagDispatchRequest` carries no input field: the steps
/// read the checkout at the well-known path and must not modify files.
fn spec_code_quick() -> DagSpec {
    DagSpec {
        max_concurrency: 4,
        name: "review-code-quick".to_string(),
        description: None,
        steps: vec![
            agent_step(
                "review-triage",
                &[],
                r#"阅读工作区 /data00/github/opencoder 的当前变更：运行 git status --short 与 git diff --stat，归纳本次代码评审的范围与主题。不要修改任何文件。以 JSON 围栏输出：{"scope": ["..."], "notes": "..."}"#,
            ),
            agent_step(
                "review-risks",
                &["review-triage"],
                r#"上游评审范围见下方上游步骤输出。针对该范围列出最多 5 个风险点，每个包含文件路径与理由。不要修改任何文件。以 JSON 围栏输出：{"risks": [{"file": "...", "why": "..."}]}"#,
            ),
            agent_step(
                "review-api-impact",
                &["review-triage"],
                r#"上游评审范围见下方上游步骤输出。从中归纳本次变更的 API 影响面：列出受影响或潜在影响的对外 API（接口、参数、响应形状、错误码、事件等），逐个说明变更点与潜在影响；无对外 API 影响时输出空数组。不要修改任何文件。以 JSON 围栏输出：{"api_impacts": [{"api": "...", "change": "...", "impact": "..."}]}"#,
            ),
            agent_step(
                "review-client",
                &["review-api-impact"],
                r#"上游 API 影响面见下方上游步骤输出。先据此关联客户端代码的设计范围：定位消费这些 API 的客户端页面、组件与交互流；再根据 API 潜在影响评审客户端代码。验收标准是 UI 交互体验不受损、符合正常交互逻辑：变更不得引入输入、点击、滑动等交互的非预期行为（例如输入未提交就被清空、点击交互失效、无法滑动），且不限于此三类。问题按三级定级：P0 业务逻辑阻断、P1 业务逻辑受损、P2 影响体验；无客户端消费方或无问题时对应数组为空。不要修改任何文件。以 JSON 围栏输出：{"client_scope": ["..."], "issues": [{"severity": "P0", "surface": "...", "why": "..."}]}"#,
            ),
            agent_step(
                "review-verdict",
                &["review-risks", "review-client"],
                r#"综合上游的范围、风险点与客户端评审结论（客户端 P0/P1 问题应计入阻断），给出评审结论与阻断项（可为空数组）。不要修改任何文件。以 JSON 围栏输出：{"verdict": "approve" 或 "changes-needed", "blocking": ["..."]}"#,
            ),
            agent_step(
                "review-report",
                &["review-verdict"],
                r#"把上游评审结论整理为一份简短 Markdown 评审报告，包含结论、阻断项与建议。不要修改任何文件。以 JSON 围栏输出：{"report_md": "..."}"#,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_fleet() -> Arc<FleetStore> {
        Arc::new(FleetStore::open_memory().await.unwrap())
    }

    fn spec_of(body: &serde_json::Value) -> opencoder_dag::DagSpec {
        opencoder_dag::decode_spec(&body["spec"])
            .unwrap_or_else(|e| panic!("seeded spec {} does not decode: {e}", body["name"]))
    }

    /// First boot: exactly the three gate names land in the fleet definition
    /// store, and every stored spec round-trips through decode + validate
    /// (proves the seeds are valid, not just inserted).
    #[tokio::test]
    async fn first_boot_inserts_three_valid_defs() {
        let fleet = memory_fleet().await;
        seed_review_dags(&fleet).await;
        let defs = fleet.definitions("dag").await.unwrap();
        let mut names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "review-code-quick",
                "review-full-acceptance",
                "review-harness-quick"
            ]
        );
        for body in &defs {
            // The control-plane definition id is the name (fleet_definitions
            // key), matching `POST /api/dag/defs` behaviour.
            assert_eq!(body["id"].as_str(), body["name"].as_str());
            let spec = spec_of(body);
            opencoder_dag::validate(&spec).unwrap_or_else(|problems| {
                panic!("seeded spec {} is invalid: {problems:?}", body["name"])
            });
        }
    }

    /// Idempotent: a second boot skips all three names; bodies stay identical.
    #[tokio::test]
    async fn re_seed_is_a_no_op() {
        let fleet = memory_fleet().await;
        seed_review_dags(&fleet).await;
        let before = fleet.definitions("dag").await.unwrap();
        seed_review_dags(&fleet).await;
        let after = fleet.definitions("dag").await.unwrap();
        assert_eq!(after.len(), 3, "re-seed must not add rows");
        // definitions() orders by id, so zip pairs the same def.
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(b, a, "re-seed must not rewrite bodies");
        }
    }

    /// Operator edits win: a def re-published under the same name survives a
    /// later boot — the seeder skips it instead of restoring the seed copy.
    #[tokio::test]
    async fn operator_edited_def_survives_re_seeding() {
        let fleet = memory_fleet().await;
        seed_review_dags(&fleet).await;
        let original = fleet
            .definitions("dag")
            .await
            .unwrap()
            .into_iter()
            .find(|d| d["name"] == "review-harness-quick")
            .unwrap();
        let edited_spec: serde_json::Value = serde_json::from_str(
            r#"{"name":"review-harness-quick","steps":[{"name":"harness-quick","kind":{"type":"wasm","command":"harness_runner.wasm quick --operator-flag"},"timeout_secs":600}]}"#,
        )
        .unwrap();
        let row_spec = edited_spec.clone();
        let mut edited = original.clone();
        edited["spec"] = edited_spec;
        edited["updated_at"] = serde_json::json!(original["updated_at"].as_i64().unwrap() + 1);
        fleet
            .put_definition("dag", "review-harness-quick", &edited)
            .await
            .unwrap();
        seed_review_dags(&fleet).await;
        let defs = fleet.definitions("dag").await.unwrap();
        assert_eq!(defs.len(), 3);
        let row = defs
            .iter()
            .find(|d| d["name"] == "review-harness-quick")
            .unwrap();
        assert_eq!(row["spec"], row_spec, "seed must not overwrite edits");
    }
}
