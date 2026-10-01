//! Catalog mutations use one connection and a transaction on MySQL.
//! StarRocks retains the project's existing sequential-write convention.
use super::Arg;
mod board;
mod io;
use crate::project::{
    tags::{reconcile_links, validate_selection, TagError},
    ProjectTag, ProjectTodoTag,
};
use crate::{ProjectInitiativePatch, ProjectTodoPatch, ProjectTodoRecord};
use anyhow::Result;
pub(super) use board::reorder_todos;
use io::{exec, query, read_link, read_tag};
pub(super) use io::{links, tags};
use sqlx::{MySqlConnection, MySqlPool};

pub(super) enum Change<'a> {
    Tag(&'a ProjectTag),
    DeleteTag(&'a str),
    CreateTodo(&'a ProjectTodoRecord, &'a [String]),
    PatchTodo(&'a str, &'a ProjectTodoPatch, Option<&'a [String]>, i64),
    PatchInitiative(&'a str, &'a ProjectInitiativePatch, i64),
    DeleteGoal(&'a str),
    DeleteInitiative(&'a str),
}

pub(super) async fn mutate(pool: &MySqlPool, starrocks: bool, change: Change<'_>) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    if !starrocks {
        sqlx::query("START TRANSACTION").execute(&mut *conn).await?;
    }
    let result = apply(&mut conn, starrocks, change).await;
    if !starrocks {
        let sql = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        sqlx::query(sql).execute(&mut *conn).await?;
    }
    result
}

async fn apply(conn: &mut MySqlConnection, sr: bool, change: Change<'_>) -> Result<bool> {
    let suffix = if sr { "" } else { " FOR UPDATE" };
    let previous = query(
        conn,
        sr,
        &format!("SELECT * FROM project_tags{suffix}"),
        &[],
    )
    .await?
    .iter()
    .map(read_tag)
    .collect::<Result<Vec<_>>>()?;
    let mut old_links = query(
        conn,
        sr,
        &format!("SELECT * FROM project_todo_tags{suffix}"),
        &[],
    )
    .await?
    .iter()
    .map(read_link)
    .collect::<Result<Vec<_>>>()?;
    let initiatives_before = query(
        conn,
        sr,
        &format!("SELECT * FROM project_initiatives{suffix}"),
        &[],
    )
    .await?
    .iter()
    .map(super::project_crud::row_to_initiative)
    .collect::<Result<Vec<_>>>()?;
    // Validate requested tags before writing, including StarRocks where transactions
    // are unavailable. A rejected request must not change the TODO payload.
    match &change {
        Change::CreateTodo(todo, ids) => {
            validate_selection(
                &previous,
                initiatives_before
                    .iter()
                    .find(|i| Some(&i.id) == todo.initiative_id.as_ref()),
                ids,
            )?;
        }
        Change::PatchTodo(id, patch, Some(ids), _) => {
            if let Some(row) = query(
                conn,
                sr,
                &format!("SELECT * FROM project_todos WHERE id=?{suffix}"),
                &[Arg::Text((*id).into())],
            )
            .await?
            .first()
            {
                let todo = super::project_crud_todo::row_to_todo(row)?;
                let parent = patch.initiative_id.as_ref().unwrap_or(&todo.initiative_id);
                validate_selection(
                    &previous,
                    initiatives_before
                        .iter()
                        .find(|i| Some(&i.id) == parent.as_ref()),
                    ids,
                )?;
            }
        }
        _ => {}
    }
    let mut selection = None;
    let changed = match change {
        Change::Tag(tag) => {
            io::validate_tag_scope(conn, sr, tag, &previous).await?;
            anyhow::ensure!(
                !previous.iter().any(|t| t.id != tag.id
                    && t.scope_type == tag.scope_type
                    && t.scope_id == tag.scope_id
                    && t.name == tag.name),
                TagError::Duplicate
            );
            if previous.iter().any(|t| t.id == tag.id) {
                exec(
                    conn,
                    sr,
                    "UPDATE project_tags SET name=? WHERE id=?",
                    vec![Arg::Text(tag.name.clone()), Arg::Text(tag.id.clone())],
                )
                .await?;
            } else {
                exec(
                    conn,
                    sr,
                    "INSERT INTO project_tags(id,scope_type,scope_id,name) VALUES (?,?,?,?)",
                    vec![
                        Arg::Text(tag.id.clone()),
                        Arg::Text(tag.scope_type.clone()),
                        Arg::Text(tag.scope_id.clone()),
                        Arg::Text(tag.name.clone()),
                    ],
                )
                .await?;
            }
            true
        }
        Change::DeleteTag(id) => {
            exec(
                conn,
                sr,
                "DELETE FROM project_tags WHERE id=?",
                vec![Arg::Text(id.into())],
            )
            .await?
                > 0
        }
        Change::CreateTodo(todo, ids) => {
            let (sql, args) = super::project_crud_todo::create_statement(todo);
            exec(conn, sr, &sql, args).await?;
            selection = Some((todo.id.as_str(), ids));
            true
        }
        Change::PatchTodo(id, patch, ids, now) => {
            let found = !query(
                conn,
                sr,
                &format!("SELECT id FROM project_todos WHERE id=?{suffix}"),
                &[Arg::Text(id.into())],
            )
            .await?
            .is_empty();
            if !found {
                return Ok(false);
            }
            let (mut sets, mut args) = super::project_crud_todo::todo_set_fragment(patch);
            sets.push("updated_at=?");
            args.extend([Arg::Int(now), Arg::Text(id.into())]);
            exec(
                conn,
                sr,
                &format!("UPDATE project_todos SET {} WHERE id=?", sets.join(",")),
                args,
            )
            .await?;
            selection = ids.map(|ids| (id, ids));
            true
        }
        Change::PatchInitiative(id, patch, now) => {
            if query(
                conn,
                sr,
                &format!("SELECT id FROM project_initiatives WHERE id=?{suffix}"),
                &[Arg::Text(id.into())],
            )
            .await?
            .is_empty()
            {
                return Ok(false);
            }
            let mut sets = vec!["updated_at=?"];
            let mut args = vec![Arg::Int(now)];
            if let Some(v) = &patch.goal_id {
                sets.push("goal_id=?");
                args.push(Arg::TextOrNull(v.clone()));
            }
            if let Some(v) = &patch.title {
                sets.push("title=?");
                args.push(Arg::Text(v.clone()));
            }
            if let Some(v) = &patch.detail_md {
                sets.push("detail_md=?");
                args.push(Arg::Text(v.clone()));
            }
            if let Some(v) = &patch.status {
                sets.push("status=?");
                args.push(Arg::Text(v.as_str().into()));
            }
            if let Some(v) = patch.sort {
                sets.push("sort_key=?");
                args.push(Arg::Int(v));
            }
            args.push(Arg::Text(id.into()));
            exec(
                conn,
                sr,
                &format!(
                    "UPDATE project_initiatives SET {} WHERE id=?",
                    sets.join(",")
                ),
                args,
            )
            .await?;
            true
        }
        Change::DeleteGoal(id) => {
            if query(
                conn,
                sr,
                &format!("SELECT id FROM project_goals WHERE id=?{suffix}"),
                &[Arg::Text(id.into())],
            )
            .await?
            .is_empty()
            {
                return Ok(false);
            }
            exec(
                conn,
                sr,
                "UPDATE project_initiatives SET goal_id=NULL WHERE goal_id=?",
                vec![Arg::Text(id.into())],
            )
            .await?;
            exec(
                conn,
                sr,
                "DELETE FROM project_tags WHERE scope_type='project' AND scope_id=?",
                vec![Arg::Text(id.into())],
            )
            .await?;
            exec(
                conn,
                sr,
                "DELETE FROM project_goals WHERE id=?",
                vec![Arg::Text(id.into())],
            )
            .await?
                > 0
        }
        Change::DeleteInitiative(id) => {
            if query(
                conn,
                sr,
                &format!("SELECT id FROM project_initiatives WHERE id=?{suffix}"),
                &[Arg::Text(id.into())],
            )
            .await?
            .is_empty()
            {
                return Ok(false);
            }
            anyhow::ensure!(
                query(
                    conn,
                    sr,
                    &format!("SELECT id FROM project_todos WHERE initiative_id=?{suffix}"),
                    &[Arg::Text(id.into())]
                )
                .await?
                .is_empty(),
                crate::project::InitiativeNotEmpty
            );
            exec(
                conn,
                sr,
                "DELETE FROM project_tags WHERE scope_type='initiative' AND scope_id=?",
                vec![Arg::Text(id.into())],
            )
            .await?;
            exec(
                conn,
                sr,
                "DELETE FROM project_initiatives WHERE id=?",
                vec![Arg::Text(id.into())],
            )
            .await?
                > 0
        }
    };
    let tags = query(conn, sr, "SELECT * FROM project_tags", &[])
        .await?
        .iter()
        .map(read_tag)
        .collect::<Result<Vec<_>>>()?;
    let initiatives = query(
        conn,
        sr,
        &format!("SELECT * FROM project_initiatives{suffix}"),
        &[],
    )
    .await?
    .iter()
    .map(super::project_crud::row_to_initiative)
    .collect::<Result<Vec<_>>>()?;
    let todos = query(
        conn,
        sr,
        &format!("SELECT * FROM project_todos{suffix}"),
        &[],
    )
    .await?
    .iter()
    .map(super::project_crud_todo::row_to_todo)
    .collect::<Result<Vec<_>>>()?;
    if let Some((id, ids)) = selection {
        let todo = todos
            .iter()
            .find(|t| t.id == id)
            .ok_or(TagError::InvalidScope)?;
        validate_selection(
            &tags,
            initiatives
                .iter()
                .find(|i| Some(&i.id) == todo.initiative_id.as_ref()),
            ids,
        )?;
        old_links.retain(|link| link.todo_id != id);
        old_links.extend(ids.iter().map(|tag_id| ProjectTodoTag {
            todo_id: id.into(),
            tag_id: tag_id.clone(),
        }));
    }
    let desired = reconcile_links(&previous, &tags, &initiatives, &todos, &old_links);
    let current = query(conn, sr, "SELECT * FROM project_todo_tags", &[])
        .await?
        .iter()
        .map(read_link)
        .collect::<Result<Vec<_>>>()?;
    for link in current.iter().filter(|link| !desired.contains(link)) {
        exec(
            conn,
            sr,
            "DELETE FROM project_todo_tags WHERE todo_id=? AND tag_id=?",
            vec![
                Arg::Text(link.todo_id.clone()),
                Arg::Text(link.tag_id.clone()),
            ],
        )
        .await?;
    }
    for link in desired.iter().filter(|link| !current.contains(link)) {
        exec(
            conn,
            sr,
            "INSERT INTO project_todo_tags(todo_id,tag_id) VALUES (?,?)",
            vec![
                Arg::Text(link.todo_id.clone()),
                Arg::Text(link.tag_id.clone()),
            ],
        )
        .await?;
    }
    Ok(changed)
}
