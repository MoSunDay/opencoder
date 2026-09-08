//! `brain` domain: capability library CRUD + target binding, semantic
//! search, dynamic plan minting/preview and dispatch. Pure `plan()` mapping
//! only; execution goes through the shared `exec_plan` transport.

use anyhow::Result;
use clap::Subcommand;

use crate::cmd::exec_plan;
use crate::cmd::raw::parse_body;
use crate::ctx::Ctx;
use crate::http::RequestPlan;

/// Parse a required `--json` value (inline JSON or `@file`).
fn required_body(raw: &str) -> Result<serde_json::Value> {
    parse_body(Some(raw))?.ok_or_else(|| anyhow::anyhow!("required --json body is empty"))
}

#[derive(Subcommand, Debug)]
pub enum BrainCmd {
    /// Capability library: CRUD + target binding.
    #[command(subcommand)]
    Caps(CapsCmd),
    /// POST /api/brain/search — {"query","k"?} nearest-neighbour search.
    Search {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// POST /api/brain/plans — {"situation","top_k"?,"model"?}.
    Plan {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// GET /api/brain/plans/{id} — one cached plan.
    PlanGet { id: String },
    /// POST /api/brain/preview — dispatch body ({"situation",...}),
    /// decision preview without executing anything.
    Preview {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// POST /api/brain/dispatch — route a situation through a plan
    /// ({"situation","plan_id"?,"top_k"?,"replan"?,"model"?}).
    Dispatch {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum CapsCmd {
    /// GET /api/brain/capabilities — the whole library.
    List,
    /// POST /api/brain/capabilities — full CapabilityInput record.
    Create {
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// GET /api/brain/capabilities/{id}.
    Get { id: String },
    /// PUT /api/brain/capabilities/{id} — full CapabilityInput rewrite.
    Update {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
    /// DELETE /api/brain/capabilities/{id}.
    Delete { id: String },
    /// GET /api/brain/capabilities/{id}/target — current binding.
    TargetGet { id: String },
    /// PUT /api/brain/capabilities/{id}/target — CapabilityTarget body.
    TargetBind {
        id: String,
        /// Body: inline JSON or @file.
        #[arg(long)]
        json: String,
    },
}

pub fn plan(sub: &BrainCmd) -> Result<RequestPlan> {
    Ok(match sub {
        BrainCmd::Caps(sub) => plan_caps(sub)?,
        BrainCmd::Search { json } => {
            RequestPlan::post("/api/brain/search").with_body(required_body(json)?)
        }
        BrainCmd::Plan { json } => {
            RequestPlan::post("/api/brain/plans").with_body(required_body(json)?)
        }
        BrainCmd::PlanGet { id } => RequestPlan::get(format!("/api/brain/plans/{id}")),
        BrainCmd::Preview { json } => {
            RequestPlan::post("/api/brain/preview").with_body(required_body(json)?)
        }
        BrainCmd::Dispatch { json } => {
            RequestPlan::post("/api/brain/dispatch").with_body(required_body(json)?)
        }
    })
}

fn plan_caps(sub: &CapsCmd) -> Result<RequestPlan> {
    Ok(match sub {
        CapsCmd::List => RequestPlan::get("/api/brain/capabilities"),
        CapsCmd::Create { json } => {
            RequestPlan::post("/api/brain/capabilities").with_body(required_body(json)?)
        }
        CapsCmd::Get { id } => RequestPlan::get(format!("/api/brain/capabilities/{id}")),
        CapsCmd::Update { id, json } => RequestPlan::put(format!("/api/brain/capabilities/{id}"))
            .with_body(required_body(json)?),
        CapsCmd::Delete { id } => RequestPlan::delete(format!("/api/brain/capabilities/{id}")),
        CapsCmd::TargetGet { id } => {
            RequestPlan::get(format!("/api/brain/capabilities/{id}/target"))
        }
        CapsCmd::TargetBind { id, json } => {
            RequestPlan::put(format!("/api/brain/capabilities/{id}/target"))
                .with_body(required_body(json)?)
        }
    })
}

pub async fn run(ctx: &Ctx, sub: BrainCmd) -> Result<i32> {
    exec_plan(ctx, plan(&sub)?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_routes_cover_full_crud_and_target() {
        let base = "/api/brain/capabilities";
        assert_eq!(plan_caps(&CapsCmd::List).unwrap(), RequestPlan::get(base));
        assert_eq!(
            plan_caps(&CapsCmd::Get { id: "c1".into() }).unwrap(),
            RequestPlan::get(format!("{base}/c1"))
        );
        assert_eq!(
            plan_caps(&CapsCmd::Delete { id: "c1".into() }).unwrap(),
            RequestPlan::delete(format!("{base}/c1"))
        );
        assert_eq!(
            plan_caps(&CapsCmd::TargetGet { id: "c1".into() }).unwrap(),
            RequestPlan::get(format!("{base}/c1/target"))
        );
        let bind = plan_caps(&CapsCmd::TargetBind {
            id: "c1".into(),
            json: r#"{"capability_id":"c1"}"#.into(),
        })
        .unwrap();
        assert_eq!(bind.method, reqwest::Method::PUT);
        assert_eq!(bind.path, format!("{base}/c1/target"));
        assert_eq!(bind.body, Some(serde_json::json!({"capability_id": "c1"})));
    }

    #[test]
    fn planning_endpoints_are_json_posts() {
        for (sub, path) in [
            (
                BrainCmd::Search {
                    json: r#"{"query":"auth"}"#.into(),
                },
                "/api/brain/search",
            ),
            (
                BrainCmd::Plan {
                    json: r#"{"situation":"deploy"}"#.into(),
                },
                "/api/brain/plans",
            ),
            (
                BrainCmd::Preview {
                    json: r#"{"situation":"deploy"}"#.into(),
                },
                "/api/brain/preview",
            ),
            (
                BrainCmd::Dispatch {
                    json: r#"{"situation":"deploy"}"#.into(),
                },
                "/api/brain/dispatch",
            ),
        ] {
            let planned = plan(&sub).unwrap();
            assert_eq!(planned.method, reqwest::Method::POST, "{path}");
            assert_eq!(planned.path, path);
            assert!(planned.body.is_some());
        }
        assert_eq!(
            plan(&BrainCmd::PlanGet { id: "p7".into() }).unwrap(),
            RequestPlan::get("/api/brain/plans/p7")
        );
    }
}
