//! System/ops surface: health, ready, time and the drain admin cycle.

use anyhow::Result;

use crate::cmd::exec_plan;
use crate::ctx::Ctx;
use crate::http::RequestPlan;
use crate::DrainAction;

pub fn plan_health() -> RequestPlan {
    RequestPlan::get("/api/health")
}

pub fn plan_ready() -> RequestPlan {
    RequestPlan::get("/api/ready")
}

pub fn plan_time() -> RequestPlan {
    RequestPlan::get("/api/time")
}

/// Drain cycle: GET status, POST freeze, DELETE reopen.
pub fn plan_drain(action: &DrainAction) -> RequestPlan {
    match action {
        DrainAction::Status => RequestPlan::get("/api/admin/drain"),
        DrainAction::Freeze => RequestPlan::post("/api/admin/drain"),
        DrainAction::Reopen => RequestPlan::delete("/api/admin/drain"),
    }
}

pub async fn health(ctx: &Ctx) -> Result<i32> {
    exec_plan(ctx, plan_health()).await
}

pub async fn ready(ctx: &Ctx) -> Result<i32> {
    exec_plan(ctx, plan_ready()).await
}

pub async fn time(ctx: &Ctx) -> Result<i32> {
    exec_plan(ctx, plan_time()).await
}

pub async fn drain(ctx: &Ctx, action: DrainAction) -> Result<i32> {
    exec_plan(ctx, plan_drain(&action)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_plans_map_methods() {
        assert_eq!(
            plan_drain(&DrainAction::Status).method,
            reqwest::Method::GET
        );
        assert_eq!(
            plan_drain(&DrainAction::Freeze).method,
            reqwest::Method::POST
        );
        assert_eq!(
            plan_drain(&DrainAction::Reopen).method,
            reqwest::Method::DELETE
        );
        assert_eq!(plan_drain(&DrainAction::Status).path, "/api/admin/drain");
    }

    #[test]
    fn probes_are_plain_gets() {
        assert_eq!(plan_health().path, "/api/health");
        assert_eq!(plan_ready().path, "/api/ready");
        assert_eq!(plan_time().path, "/api/time");
    }
}
