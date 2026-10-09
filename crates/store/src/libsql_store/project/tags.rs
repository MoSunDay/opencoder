//! Transactional tag catalog and TODO selection changes.
use crate::project::{
    tags::{reconcile_links, validate_selection, TagError},
    ProjectTag, ProjectTodoTag,
};
use crate::{ProjectTodoPatch, ProjectTodoRecord};
use anyhow::Result;
use libsql::{params, Connection};

pub(super) async fn list(conn: &Connection) -> Result<Vec<ProjectTag>> {
    let mut rows = conn
        .query(
            "SELECT id,scope_type,scope_id,name FROM project_tags ORDER BY name,id",
            (),
        )
        .await?;
    let mut result = vec![];
    while let Some(row) = rows.next().await? {
        result.push(ProjectTag {
            id: row.get(0)?,
            scope_type: row.get(1)?,
            scope_id: row.get(2)?,
            name: row.get(3)?,
        });
    }
    Ok(result)
}

pub(super) async fn links(conn: &Connection) -> Result<Vec<ProjectTodoTag>> {
    let mut rows = conn
        .query(
            "SELECT todo_id,tag_id FROM project_todo_tags ORDER BY todo_id,tag_id",
            (),
        )
        .await?;
    let mut result = vec![];
    while let Some(row) = rows.next().await? {
        result.push(ProjectTodoTag {
            todo_id: row.get(0)?,
            tag_id: row.get(1)?,
        });
    }
    Ok(result)
}

pub(super) async fn reconcile(
    conn: &Connection,
    previous: &[ProjectTag],
    selection: Option<(&str, &[String])>,
) -> Result<()> {
    let tags = list(conn).await?;
    let initiatives = super::list_initiatives(conn, None).await?;
    let todos = super::super::project_runs::list_todos(conn, None).await?;
    let mut old_links = links(conn).await?;
    if let Some((todo_id, ids)) = selection {
        let todo = todos
            .iter()
            .find(|t| t.id == todo_id)
            .ok_or(TagError::InvalidScope)?;
        let initiative = initiatives
            .iter()
            .find(|i| todo.initiative_id.as_ref() == Some(&i.id));
        validate_selection(&tags, initiative, ids)?;
        old_links.retain(|link| link.todo_id != todo_id);
        old_links.extend(ids.iter().map(|id| ProjectTodoTag {
            todo_id: todo_id.into(),
            tag_id: id.clone(),
        }));
    }
    let desired = reconcile_links(previous, &tags, &initiatives, &todos, &old_links);
    let current = links(conn).await?;
    for link in current.iter().filter(|link| !desired.contains(link)) {
        conn.execute(
            "DELETE FROM project_todo_tags WHERE todo_id=? AND tag_id=?",
            params![link.todo_id.as_str(), link.tag_id.as_str()],
        )
        .await?;
    }
    for link in desired.iter().filter(|link| !current.contains(link)) {
        conn.execute(
            "INSERT INTO project_todo_tags(todo_id,tag_id) VALUES (?,?)",
            params![link.todo_id.as_str(), link.tag_id.as_str()],
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn write(conn: &Connection, tag: &ProjectTag) -> Result<()> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let previous = list(conn).await?;
        let table = match tag.scope_type.as_str() {
            "project" => "project_goals",
            "initiative" => "project_initiatives",
            _ => anyhow::bail!(TagError::InvalidScope),
        };
        let mut owner = conn
            .query(
                &format!("SELECT id FROM {table} WHERE id=?"),
                params![tag.scope_id.as_str()],
            )
            .await?;
        anyhow::ensure!(owner.next().await?.is_some(), TagError::InvalidScope);
        drop(owner);
        anyhow::ensure!(
            !previous.iter().any(|t| t.id != tag.id
                && t.scope_type == tag.scope_type
                && t.scope_id == tag.scope_id
                && t.name == tag.name),
            TagError::Duplicate
        );
        if previous.iter().any(|t| t.id == tag.id) {
            conn.execute(
                "UPDATE project_tags SET name=?,scope_type=?,scope_id=? WHERE id=?",
                params![
                    tag.name.as_str(),
                    tag.scope_type.as_str(),
                    tag.scope_id.as_str(),
                    tag.id.as_str()
                ],
            )
            .await?;
        } else {
            conn.execute(
                "INSERT INTO project_tags(id,scope_type,scope_id,name) VALUES (?,?,?,?)",
                params![
                    tag.id.as_str(),
                    tag.scope_type.as_str(),
                    tag.scope_id.as_str(),
                    tag.name.as_str()
                ],
            )
            .await?;
        }
        reconcile(conn, &previous, None).await
    })
    .await
}

pub(super) async fn delete(conn: &Connection, id: &str) -> Result<bool> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let previous = list(conn).await?;
        let changed = conn
            .execute("DELETE FROM project_tags WHERE id=?", params![id])
            .await?
            > 0;
        reconcile(conn, &previous, None).await?;
        Ok(changed)
    })
    .await
}

pub(super) async fn create_todo(
    conn: &Connection,
    todo: &ProjectTodoRecord,
    ids: &[String],
) -> Result<()> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        super::super::project_runs::create_todo(conn, todo).await?;
        reconcile(conn, &[], Some((&todo.id, ids))).await
    })
    .await
}

pub(super) async fn patch_todo(
    conn: &Connection,
    id: &str,
    patch: &ProjectTodoPatch,
    ids: Option<&[String]>,
    now: i64,
) -> Result<bool> {
    super::super::tx::run_tx(conn, "BEGIN IMMEDIATE", || async move {
        let changed = super::super::project_runs::patch_todo(conn, id, patch, now).await?;
        if changed && (patch.initiative_id.is_some() || ids.is_some()) {
            reconcile(conn, &[], ids.map(|ids| (id, ids))).await?;
        }
        Ok(changed)
    })
    .await
}
