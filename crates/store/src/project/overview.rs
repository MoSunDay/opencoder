//! Shared, pure overview projection for standalone Web and fleet control.
use crate::{ProjectGoalRecord, ProjectMilestoneRecord};
use serde_json::{json, Value};

pub fn overview(
    goals: &[ProjectGoalRecord],
    milestones: &[ProjectMilestoneRecord],
    todos: &[Value],
) -> Value {
    let milestone = |m: &ProjectMilestoneRecord| {
        let mut value = json!(m);
        value["todos"] = json!(todos
            .iter()
            .filter(|t| t["milestone_id"] == m.id)
            .collect::<Vec<_>>());
        value
    };
    let nested: Vec<_> = goals
        .iter()
        .map(|g| {
            let mut value = json!(g);
            value["milestones"] = json!(milestones
                .iter()
                .filter(|m| m.goal_id.as_deref() == Some(g.id.as_str()))
                .map(&milestone)
                .collect::<Vec<_>>());
            value
        })
        .collect();
    json!({
        "goals": nested,
        "standalone_milestones": milestones.iter().filter(|m| m.goal_id.is_none()).map(milestone).collect::<Vec<_>>(),
        "backlog": todos.iter().filter(|t| t["milestone_id"].is_null()).collect::<Vec<_>>(),
    })
}
