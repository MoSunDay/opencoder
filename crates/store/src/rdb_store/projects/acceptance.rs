//! Acceptance precondition check, staged before the TODO patch it guards.

use anyhow::Result;

use crate::project::acceptance::{AcceptanceConflict, TodoAcceptance};
use crate::rdb_store::projects::{initiatives, links, todos};

/// Every observed fact the patch relies on must still hold: the todo row
/// itself, its project membership, the latest linked execution, each
/// required-done dependency and (optionally) all other todos in the project.
pub fn check(store: &rdb::store::Store, id: &str, expected: &TodoAcceptance) -> Result<()> {
    let todo = todos::get(store, id)?;
    anyhow::ensure!(
        todo.as_ref()
            .is_some_and(|todo| expected.matches_todo(todo)),
        AcceptanceConflict
    );
    let todo = todo.expect("checked above");
    let goal = todo
        .initiative_id
        .as_deref()
        .and_then(|initiative| initiatives::get(store, initiative).ok().flatten())
        .and_then(|initiative| initiative.goal_id);
    anyhow::ensure!(
        goal.as_deref() == Some(expected.project_id.as_str()),
        AcceptanceConflict
    );
    let latest = links::list(store, id)?
        .into_iter()
        .max_by(|a, b| (a.created_at, &a.execution_id).cmp(&(b.created_at, &b.execution_id)));
    anyhow::ensure!(
        latest.is_some_and(|link| link.execution_id == expected.execution_id
            && link.capability_id.as_deref() == Some(expected.capability_id.as_str())),
        AcceptanceConflict
    );
    for dependency in &expected.required_done {
        anyhow::ensure!(dependency != id, AcceptanceConflict);
        let row = todos::get(store, dependency)?;
        let goal = row.as_ref().and_then(|dep| {
            dep.initiative_id
                .as_deref()
                .and_then(|initiative| initiatives::get(store, initiative).ok().flatten())
                .and_then(|initiative| initiative.goal_id)
        });
        anyhow::ensure!(
            row.as_ref().is_some_and(|dep| dep.board_status == "done")
                && goal.as_deref() == Some(expected.project_id.as_str()),
            AcceptanceConflict
        );
    }
    if expected.all_other_todos {
        for other in todos::list_all(store)? {
            if other.id == id || other.board_status == "done" {
                continue;
            }
            let goal = other
                .initiative_id
                .as_deref()
                .and_then(|initiative| initiatives::get(store, initiative).ok().flatten())
                .and_then(|initiative| initiative.goal_id);
            anyhow::ensure!(
                goal.as_deref() != Some(expected.project_id.as_str()),
                AcceptanceConflict
            );
        }
    }
    Ok(())
}
