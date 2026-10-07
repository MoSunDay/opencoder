//! Shared snapshot and pure progress projection for standalone Web and control.
use crate::{ProjectGoalRecord, ProjectInitiativeRecord, ProjectStore};
use serde_json::{json, Value};

pub fn overview(
    goals: &[ProjectGoalRecord],
    initiatives: &[ProjectInitiativeRecord],
    todos: &[Value],
) -> Value {
    let progress = |rows: &[Value]| {
        json!({
            "total": rows.len(),
            "done": rows.iter().filter(|todo| todo["board_status"] == "done").count(),
        })
    };
    let group = |i: &ProjectInitiativeRecord| {
        let rows: Vec<Value> = todos
            .iter()
            .filter(|t| t["initiative_id"] == i.id)
            .cloned()
            .collect();
        let mut value = json!(i);
        value["progress"] = progress(&rows);
        value["todos"] = json!(rows);
        value
    };
    let nested: Vec<_> = goals
        .iter()
        .map(|g| {
            let groups: Vec<_> = initiatives
                .iter()
                .filter(|i| i.goal_id.as_deref() == Some(g.id.as_str()))
                .map(&group)
                .collect();
            let rows: Vec<_> = groups
                .iter()
                .flat_map(|i| i["todos"].as_array().unwrap().clone())
                .collect();
            let mut value = json!(g);
            value["progress"] = progress(&rows);
            value["initiatives"] = json!(groups);
            value
        })
        .collect();
    json!({ "goals": nested,
        "standalone_initiatives": initiatives.iter().filter(|i| !goals.iter().any(|g| i.goal_id.as_deref() == Some(g.id.as_str()))).map(&group).collect::<Vec<_>>(),
        "backlog": todos.iter().filter(|t| !initiatives.iter().any(|i| t["initiative_id"] == i.id)).map(|t| { let mut value=t.clone(); value["initiative_id"]=Value::Null; value }).collect::<Vec<_>>(),
    })
}

pub async fn load(store: &dyn ProjectStore) -> anyhow::Result<Value> {
    let goals = store.list_goals().await?;
    let initiatives = store.list_initiatives(None).await?;
    let tags = store.list_tags().await?;
    let links = store.list_todo_tags().await?;
    let assignments = store.latest_todo_assignments().await?;
    let todos: Vec<Value> = store
        .list_todos(None)
        .await?
        .into_iter()
        .map(|todo| {
            let mut value = json!(todo);
            value["tag_ids"] = json!(links
                .iter()
                .filter(|link| link.todo_id == todo.id)
                .map(|link| &link.tag_id)
                .collect::<Vec<_>>());
            if let Some(assignment) = assignments.iter().find(|a| a.todo_id == todo.id) {
                value["latest_assignment"] = json!(assignment);
            }
            value
        })
        .collect();
    let mut result = overview(&goals, &initiatives, &todos);
    result["tags"] = json!(tags);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_uses_manual_board_completion() {
        let i = ProjectInitiativeRecord {
            id: "i".into(),
            goal_id: None,
            title: "I".into(),
            detail_md: None,
            status: crate::ProjectInitiativeStatus::Planned,
            sort: 0,
            created_at: 0,
            updated_at: 0,
        };
        let todos = vec![
            json!({"initiative_id":"i","board_status":"todo","status":"done"}),
            json!({"initiative_id":"i","board_status":"done","status":"draft"}),
        ];
        let result = overview(&[], &[i], &todos);
        assert_eq!(
            result["standalone_initiatives"][0]["progress"],
            json!({"total":2,"done":1})
        );
        assert_eq!(overview(&[], &[], &[])["backlog"], json!([]));
    }
}
