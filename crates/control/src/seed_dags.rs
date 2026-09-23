//! Startup seeding of the built-in review release-gate DAG definitions.
//!
//! WHY: a fresh install otherwise exposes an empty `dag_defs` table, so the
//! quick harness gate is invisible until somebody hand-posts its spec. Seeding
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

/// Seed the quick harness release-gate definition into the fleet definition store
/// (`fleet_definitions`, the store behind `GET/POST /api/dag/defs`).
/// Idempotent by name; errors are logged, never propagated: the server stays
/// useful without the seeds.
pub(crate) async fn seed_review_dags(fleet: &Arc<FleetStore>) {
    let specs = [spec_harness_quick()];
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
        trigger_rule: Default::default(),
        name: name.to_string(),
        depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
        kind: StepKind::Wasm {
            command: command.to_string(),
            sandbox: None,
        },
        timeout_secs: Some(timeout_secs),
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

    /// First boot: only the retained quick harness name land in the fleet definition
    /// store, and every stored spec round-trips through decode + validate
    /// (proves the seeds are valid, not just inserted).
    #[tokio::test]
    async fn first_boot_inserts_only_retained_harness() {
        let fleet = memory_fleet().await;
        seed_review_dags(&fleet).await;
        let defs = fleet.definitions("dag").await.unwrap();
        let mut names: Vec<&str> = defs.iter().filter_map(|d| d["name"].as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["review-harness-quick"]);
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

    /// Idempotent: a second boot skips the existing name; bodies stay identical.
    #[tokio::test]
    async fn re_seed_is_a_no_op() {
        let fleet = memory_fleet().await;
        seed_review_dags(&fleet).await;
        let before = fleet.definitions("dag").await.unwrap();
        seed_review_dags(&fleet).await;
        let after = fleet.definitions("dag").await.unwrap();
        assert_eq!(after.len(), 1, "re-seed must not add rows");
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
        assert_eq!(defs.len(), 1);
        let row = defs
            .iter()
            .find(|d| d["name"] == "review-harness-quick")
            .unwrap();
        assert_eq!(row["spec"], row_spec, "seed must not overwrite edits");
    }
}
