//! `schedule` domain: the control-plane cron scheduler (`schedules.json`
//! definitions + `/api/schedules` fire history). Pure `plan()` mapping;
//! execution goes through the shared `exec_plan` transport.

use anyhow::Result;
use clap::Subcommand;

use crate::ctx::Ctx;
use crate::http::RequestPlan;

#[derive(Subcommand, Debug)]
pub enum ScheduleCmd {
    /// List configured schedules with their latest fire and next tick.
    #[command(alias = "ls")]
    List,
    /// Fire history of one schedule, newest tick first.
    Runs {
        id: String,
        /// Maximum rows (1-500, default 50).
        #[arg(long)]
        limit: Option<u32>,
    },
}

pub fn plan(sub: &ScheduleCmd) -> Result<RequestPlan> {
    Ok(match sub {
        ScheduleCmd::List => RequestPlan::get("/api/schedules"),
        ScheduleCmd::Runs { id, limit } => {
            let mut plan = RequestPlan::get(format!("/api/schedules/{id}/runs"));
            if let Some(limit) = limit {
                plan = plan.with("limit", limit.to_string());
            }
            plan
        }
    })
}

pub async fn run(ctx: &Ctx, sub: ScheduleCmd) -> Result<i32> {
    let request = plan(&sub)?;
    crate::cmd::exec_plan(ctx, request).await
}
