//! Shared, pure overview projection for standalone Web and fleet control.
use crate::{ProjectGoalRecord, ProjectMilestoneRecord};
use serde_json::{json, Value};

pub fn overview(
    goals: &[ProjectGoalRecord],
    milestones: &[ProjectMilestoneRecord],
    initiatives: &[ProjectMilestoneRecord],
    todos: &[Value],
) -> Value {
    let group = |m: &ProjectMilestoneRecord| {
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
                .map(&group)
                .collect::<Vec<_>>());
            value["initiatives"] = json!(initiatives
                .iter()
                .filter(|initiative| initiative.goal_id.as_deref() == Some(g.id.as_str()))
                .map(&group)
                .collect::<Vec<_>>());
            value
        })
        .collect();
    json!({
        "goals": nested,
        "standalone_milestones": milestones.iter().filter(|m| m.goal_id.is_none()).map(&group).collect::<Vec<_>>(),
        "standalone_initiatives": initiatives.iter().filter(|item| item.goal_id.is_none()).map(&group).collect::<Vec<_>>(),
        "backlog": todos.iter().filter(|t| t["milestone_id"].is_null()).collect::<Vec<_>>(),
    })
}
